// mavis_core/src/safety/rollback.rs
// Undo for destructive shell commands: copy the files a command names
// before it runs, keep the copy for five minutes, put it back on "undo".
// "Where possible" — a command this can't read with certainty gets no copy.

use anyhow::{bail, Context, Result};
use log::{info, warn};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a saved copy can be restored, and how long it is kept.
pub const UNDO_WINDOW: Duration = Duration::from_secs(5 * 60);

/// A copy larger than this isn't made; the command still runs.
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const MAX_FILES: usize = 5_000;

/// Commands whose arguments are the files they change.
const FILE_COMMANDS: &[&str] = &["rm", "rmdir", "mv", "shred", "truncate"];

#[derive(Clone)]
pub struct Rollback {
    root: PathBuf,
}

impl Rollback {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Save what `command` is about to change. Returns how many paths were
    /// saved; None when there was nothing to save or it couldn't be done.
    pub fn snapshot(&self, command: &str) -> Option<usize> {
        self.prune();
        let cwd = std::env::current_dir().ok()?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let paths = targets(command, &cwd, home.as_deref())?;
        if paths.is_empty() {
            return None;
        }

        let dir = self.root.join(now_millis().to_string());
        match save(&paths, &dir) {
            Ok(()) => Some(paths.len()),
            Err(e) => {
                warn!("Rollback: no undo for this command: {}", e);
                let _ = std::fs::remove_dir_all(&dir);
                None
            }
        }
    }

    /// Put back the most recent copy still inside the window, then discard
    /// it. Returns the restored paths; empty when there is nothing to undo.
    pub fn undo(&self) -> Result<Vec<String>> {
        self.prune();
        let Some(dir) = self.snapshots().into_iter().max() else {
            return Ok(Vec::new());
        };
        let dir = self.root.join(dir.to_string());
        let manifest: Vec<(String, String)> =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json"))?)?;

        let mut restored = Vec::new();
        for (original, saved) in manifest {
            let original = PathBuf::from(original);
            if let Some(parent) = original.parent() {
                std::fs::create_dir_all(parent)?;
            }
            copy_tree(&dir.join(saved), &original, &mut Budget::unlimited())
                .with_context(|| format!("restoring {}", original.display()))?;
            restored.push(original.display().to_string());
        }
        std::fs::remove_dir_all(&dir)?;
        info!("Rollback: restored {} path(s)", restored.len());
        Ok(restored)
    }

    /// Delete copies older than the window. They are the user's files;
    /// they shouldn't outlive their purpose.
    pub fn prune(&self) {
        self.prune_at(now_millis());
    }

    fn prune_at(&self, now: u128) {
        for stamp in self.snapshots() {
            if now.saturating_sub(stamp) > UNDO_WINDOW.as_millis() {
                let _ = std::fs::remove_dir_all(self.root.join(stamp.to_string()));
            }
        }
    }

    /// Snapshot directories, named by the millisecond they were taken.
    fn snapshots(&self) -> Vec<u128> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect()
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// The existing paths a simple file command names. None if the command is
/// anything else, or uses shell syntax whose effect can't be known here —
/// globs, variables, pipes, several commands.
pub fn targets(command: &str, cwd: &Path, home: Option<&Path>) -> Option<Vec<PathBuf>> {
    let words = split(command)?;
    let (program, args) = words.split_first()?;
    if !FILE_COMMANDS.contains(&program.as_str()) {
        return None;
    }

    let mut paths = Vec::new();
    let mut flags_done = false;
    for arg in args {
        if arg == "--" {
            flags_done = true;
        } else if flags_done || !arg.starts_with('-') {
            let path = match (arg.strip_prefix("~/"), home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => cwd.join(arg),
            };
            // Only what exists can be saved; "0" in `truncate -s 0 f` isn't a file.
            if path.symlink_metadata().is_ok() {
                paths.push(path);
            }
        }
    }
    Some(paths)
}

/// Split a command into words, honouring plain quotes. None on any shell
/// syntax that could change what the words mean.
fn split(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;

    for c in command.chars() {
        match quote {
            Some('\'') if c == '\'' => quote = None,
            Some('"') if c == '"' => quote = None,
            Some('\'') => word.push(c),
            Some(_) if "$`\\".contains(c) => return None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None if "|&;<>$`(){}*?[]!#\\".contains(c) => return None,
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

struct Budget {
    bytes: u64,
    files: usize,
}

impl Budget {
    fn unlimited() -> Self {
        Self { bytes: u64::MAX, files: usize::MAX }
    }

    fn spend(&mut self, bytes: u64) -> Result<()> {
        if self.files == 0 || self.bytes < bytes {
            bail!("too large to copy");
        }
        self.files -= 1;
        self.bytes -= bytes;
        Ok(())
    }
}

/// Copy `paths` into `dir`, with a manifest of where each came from.
fn save(paths: &[PathBuf], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut budget = Budget { bytes: MAX_BYTES, files: MAX_FILES };
    let mut manifest = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        copy_tree(path, &dir.join(i.to_string()), &mut budget)?;
        manifest.push((path.display().to_string(), i.to_string()));
    }
    std::fs::write(dir.join("manifest.json"), serde_json::to_string(&manifest)?)?;
    Ok(())
}

/// Copy a file, directory or symlink. Symlinks are copied as links, never
/// followed, so a copy can't escape the tree it was asked for.
fn copy_tree(from: &Path, to: &Path, budget: &mut Budget) -> Result<()> {
    let meta = from.symlink_metadata()?;
    // Never write through a link that now sits where the file was.
    if to.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(to)?;
    }
    if meta.file_type().is_symlink() {
        budget.spend(0)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()), budget)?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
    } else {
        budget.spend(meta.len())?;
        std::fs::copy(from, to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mavis_rollback_{}_{}_{}", tag, std::process::id(), now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(command: &str, cwd: &Path) -> Option<Vec<String>> {
        targets(command, cwd, None).map(|paths| {
            paths
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect()
        })
    }

    #[test]
    fn simple_file_commands_name_their_files() {
        let dir = temp_dir("targets");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("my notes.txt"), "b").unwrap();

        assert_eq!(names("rm -f a.txt missing.txt", &dir).unwrap(), ["a.txt"]);
        assert_eq!(names("rm 'my notes.txt'", &dir).unwrap(), ["my notes.txt"]);
        assert_eq!(names("mv a.txt \"my notes.txt\"", &dir).unwrap(), ["a.txt", "my notes.txt"]);
        assert_eq!(names("truncate -s 0 a.txt", &dir).unwrap(), ["a.txt"]);
        assert_eq!(names("rm -- a.txt", &dir).unwrap(), ["a.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Anything whose effect can't be read off the command gets no copy.
    #[test]
    fn shell_syntax_and_other_commands_are_left_alone() {
        let dir = temp_dir("unsure");
        for command in [
            "rm *.txt",
            "rm $HOME/x",
            "rm a.txt; rm b.txt",
            "rm a.txt && echo done",
            "cat a.txt | xargs rm",
            "rm `which x`",
            "rm \"$X\"",
            "rm 'unterminated",
            "sudo rm a.txt",
            "git clean -fd",
            "ls",
            "",
        ] {
            assert!(targets(command, &dir, None).is_none(), "{}", command);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_file_and_folder_come_back() {
        let dir = temp_dir("undo");
        let file = dir.join("notes.txt");
        let folder = dir.join("project");
        std::fs::write(&file, "important").unwrap();
        std::fs::create_dir_all(folder.join("src")).unwrap();
        std::fs::write(folder.join("src/main.rs"), "fn main() {}").unwrap();

        let rollback = Rollback::new(dir.join(".mavis-backup"));
        let paths = vec![file.clone(), folder.clone()];
        save(&paths, &rollback.root.join(now_millis().to_string())).unwrap();

        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();

        let restored = rollback.undo().unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "important");
        assert_eq!(std::fs::read_to_string(folder.join("src/main.rs")).unwrap(), "fn main() {}");

        assert!(rollback.undo().unwrap().is_empty(), "an undo is used once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_older_than_the_window_are_deleted_not_restored() {
        let dir = temp_dir("prune");
        let file = dir.join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let rollback = Rollback::new(dir.join(".mavis-backup"));
        save(&[file], &rollback.root.join(now_millis().to_string())).unwrap();

        rollback.prune_at(now_millis() + UNDO_WINDOW.as_millis() - 1_000);
        assert_eq!(rollback.snapshots().len(), 1, "still inside the window");
        rollback.prune_at(now_millis() + UNDO_WINDOW.as_millis() + 1_000);
        assert!(rollback.snapshots().is_empty());
        assert!(rollback.undo().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_that_would_be_too_large_is_not_made() {
        let dir = temp_dir("budget");
        let file = dir.join("big");
        std::fs::write(&file, vec![0u8; 4096]).unwrap();
        let mut budget = Budget { bytes: 1024, files: 10 };
        assert!(copy_tree(&file, &dir.join("copy"), &mut budget).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_copied_as_links_not_followed() {
        let dir = temp_dir("links");
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        let tree = dir.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("link")).unwrap();

        let copy = dir.join("copy");
        copy_tree(&tree, &copy, &mut Budget::unlimited()).unwrap();
        assert!(copy.join("link").symlink_metadata().unwrap().file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&dir);
    }
}