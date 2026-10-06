// mavis_core/src/sentinel/checks.rs
// Checks that have to run an external tool: file integrity and advisories
// (step 4), and the Windows and macOS inventories (step 5). Each yields a
// snapshot that is diffed against the last one, like a privilege surface.

use super::change::Change;
use super::packages::PackageManager;
use super::{advisories, integrity, inventory};
use chrono::{DateTime, Utc};
use log::{info, warn};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

pub type Snapshot = BTreeMap<String, String>;

const DAILY: Duration = Duration::from_secs(24 * 3600);
const HOURLY: Duration = Duration::from_secs(3600);

/// After a check fails, wait this long before trying it again.
pub const RETRY: Duration = Duration::from_secs(3600);

pub struct Output {
    /// The tool exited with status 0.
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

enum Action {
    Run { program: PathBuf, args: Vec<String> },
    Read(Arc<dyn Fn() -> Option<Snapshot> + Send + Sync>),
}

/// How a check's snapshots turn into changes.
enum Kind {
    Integrity,
    Advisories { scanner: &'static str },
    Apps,
    Autostart,
    Detections { scanner: &'static str },
}

pub struct Check {
    /// Names the snapshot in the store and the changes it produces.
    pub source: &'static str,
    pub every: Duration,
    pub timeout: Duration,
    /// Its result is only valid if no package transaction ran meanwhile.
    pub needs_idle_packages: bool,
    action: Action,
    parse: fn(&Output) -> Option<Snapshot>,
    kind: Kind,
}

impl Check {
    /// Run it. None means "no result this time" — never an empty result.
    pub async fn snapshot(&self) -> Option<Snapshot> {
        match &self.action {
            Action::Read(read) => read(),
            Action::Run { program, args } => {
                let output = run(program, args, self.timeout).await?;
                let snapshot = (self.parse)(&output);
                if snapshot.is_none() {
                    warn!("Sentinel: {} check gave no usable result", self.source);
                }
                snapshot
            }
        }
    }

    pub fn diff(&self, old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
        match self.kind {
            Kind::Integrity => integrity::diff(old, new, at),
            Kind::Advisories { scanner } => advisories::diff(scanner, old, new, at),
            Kind::Apps => inventory::diff_apps(self.source, old, new, at),
            Kind::Autostart => inventory::diff_autostart(self.source, old, new, at),
            Kind::Detections { scanner } => inventory::diff_detections(self.source, scanner, old, new, at),
        }
    }
}

/// Advisory scanners download their data, so they need their own opt-in
/// on top of MAVIS_SENTINEL.
pub fn advisories_enabled() -> bool {
    matches!(
        std::env::var("MAVIS_SENTINEL_ADVISORIES").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Run the daily checks at startup even if they ran recently. For testing.
pub fn check_now() -> bool {
    matches!(
        std::env::var("MAVIS_SENTINEL_CHECK_NOW").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// The checks that apply to this machine and whose tool is installed.
pub fn for_this_machine(manager: PackageManager) -> Vec<Check> {
    let mut wanted: Vec<(&'static str, Option<Check>)> = Vec::new();

    match manager {
        PackageManager::Pacman => {
            // Findings go to stderr and summaries to stdout; exit status is
            // 1 whenever anything differs, so it says nothing about success.
            wanted.push((
                "pacman",
                command(integrity::SOURCE, "pacman", &["-Qkk"], Kind::Integrity, |o| {
                    integrity::parse_pacman_qkk(&format!("{}\n{}", o.stdout, o.stderr))
                }),
            ));
            if advisories_enabled() {
                let args = ["--color", "never", "--format", advisories::ARCH_AUDIT_FORMAT];
                let kind = Kind::Advisories { scanner: "arch-audit" };
                wanted.push((
                    "arch-audit",
                    command(advisories::SOURCE, "arch-audit", &args, kind, |o| {
                        o.ok.then(|| advisories::parse_arch_audit(&o.stdout))
                    }),
                ));
            }
        }
        PackageManager::Rpm => wanted.push((
            "rpm",
            // Exit status is 1 on any mismatch; a real failure says "error:".
            command(integrity::SOURCE, "rpm", &["-Va"], Kind::Integrity, |o| {
                let failed = o.stderr.lines().any(|l| l.starts_with("error:"));
                (!failed).then(|| integrity::parse_verify(&o.stdout))
            }),
        )),
        PackageManager::Dpkg => {
            // Same md5sums check as debsums, without needing it installed.
            wanted.push((
                "dpkg",
                command(integrity::SOURCE, "dpkg", &["--verify"], Kind::Integrity, |o| {
                    o.ok.then(|| integrity::parse_verify(&o.stdout))
                }),
            ));
            // debsecan reads Debian's tracker; on Ubuntu its answers are wrong.
            let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            if advisories_enabled() && os_release(&release, "ID").as_deref() == Some("debian") {
                let suite = os_release(&release, "VERSION_CODENAME");
                let args: Vec<&str> = match &suite {
                    Some(s) => vec!["--suite", s.as_str()],
                    None => vec![],
                };
                let kind = Kind::Advisories { scanner: "debsecan" };
                wanted.push((
                    "debsecan",
                    command(advisories::SOURCE, "debsecan", &args, kind, |o| {
                        o.ok.then(|| advisories::parse_debsecan(&o.stdout))
                    }),
                ));
            }
        }
        PackageManager::Unknown => {}
    }

    if cfg!(target_os = "windows") {
        let ps = |script: String| ["-NoProfile".to_string(), "-NonInteractive".into(), "-Command".into(), script];
        let hourly = |c: Option<Check>| c.map(|c| Check { every: HOURLY, needs_idle_packages: false, ..c });
        wanted.push((
            "powershell",
            hourly(command_owned("apps", "powershell", &ps(inventory::windows_apps_script()), Kind::Apps, |o| {
                inventory::parse_marked(&o.stdout)
            })),
        ));
        wanted.push((
            "powershell",
            hourly(command_owned(
                "autostart",
                "powershell",
                &ps(inventory::windows_autostart_script()),
                Kind::Autostart,
                |o| inventory::parse_marked_pairs(&o.stdout),
            )),
        ));
        let kind = Kind::Detections { scanner: "Microsoft Defender" };
        wanted.push((
            "powershell",
            hourly(command_owned("defender", "powershell", &ps(inventory::windows_defender_script()), kind, |o| {
                inventory::parse_marked(&o.stdout)
            })),
        ));
    }

    if cfg!(target_os = "macos") {
        let hourly = |c: Option<Check>| c.map(|c| Check { every: HOURLY, needs_idle_packages: false, ..c });
        wanted.push((
            "brew",
            hourly(command("homebrew", "brew", &["list", "--versions"], Kind::Apps, |o| {
                o.ok.then(|| inventory::parse_brew(&o.stdout))
            })),
        ));
        wanted.push((
            "pkgutil",
            hourly(command("pkgutil", "pkgutil", &["--pkgs"], Kind::Apps, |o| {
                o.ok.then(|| inventory::parse_pkgutil(&o.stdout))
            })),
        ));
        wanted.push((
            "launchd",
            Some(Check {
                source: "launchd",
                every: HOURLY,
                timeout: Duration::from_secs(60),
                needs_idle_packages: false,
                action: Action::Read(Arc::new(inventory::read_launchd)),
                parse: |_| None,
                kind: Kind::Autostart,
            }),
        ));
    }

    wanted
        .into_iter()
        .filter_map(|(tool, check)| {
            if check.is_none() {
                info!("Sentinel: {} is not installed — that check is off", tool);
            }
            check
        })
        .collect()
}

/// An integrity check fed by `read` instead of a tool, always due.
#[cfg(test)]
pub fn fake_integrity(
    needs_idle_packages: bool,
    read: impl Fn() -> Option<Snapshot> + Send + Sync + 'static,
) -> Check {
    Check {
        source: integrity::SOURCE,
        every: Duration::ZERO,
        timeout: Duration::from_secs(5),
        needs_idle_packages,
        action: Action::Read(Arc::new(read)),
        parse: |_| None,
        kind: Kind::Integrity,
    }
}

/// An app inventory fed by `read`, always due.
#[cfg(test)]
pub fn fake_apps(read: impl Fn() -> Option<Snapshot> + Send + Sync + 'static) -> Check {
    Check {
        source: "apps",
        kind: Kind::Apps,
        ..fake_integrity(false, read)
    }
}

/// A daily check that runs `program`; None if it isn't installed.
fn command(
    source: &'static str,
    program: &str,
    args: &[&str],
    kind: Kind,
    parse: fn(&Output) -> Option<Snapshot>,
) -> Option<Check> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    command_owned(source, program, &args, kind, parse)
}

fn command_owned(
    source: &'static str,
    program: &str,
    args: &[String],
    kind: Kind,
    parse: fn(&Output) -> Option<Snapshot>,
) -> Option<Check> {
    Some(Check {
        source,
        every: DAILY,
        // A full verify reads every packaged file; allow for a slow disk.
        timeout: Duration::from_secs(15 * 60),
        needs_idle_packages: matches!(kind, Kind::Integrity),
        action: Action::Run {
            program: find_program(program)?,
            args: args.to_vec(),
        },
        parse,
        kind,
    })
}

/// A value from /etc/os-release, unquoted.
fn os_release(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// Look on PATH, then where Homebrew lives — a desktop app's PATH on
/// macOS often lacks it.
fn find_program(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{}{}", name, ext))))
        .find(|candidate| candidate.is_file())
}

/// Run a tool quietly: lowest priority, English output, a time limit.
/// None if it couldn't be run, was killed, or ran out of time.
async fn run(program: &Path, args: &[String], limit: Duration) -> Option<Output> {
    let mut cmd = if cfg!(unix) {
        let mut c = tokio::process::Command::new("nice");
        c.args(["-n", "19"]).arg(program);
        c
    } else {
        tokio::process::Command::new(program)
    };
    cmd.args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Dropping the future on timeout or shutdown kills the tool too.
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let name = program.display();
    match tokio::time::timeout(limit, cmd.output()).await {
        Ok(Ok(out)) if out.status.code().is_some() => Some(Output {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
        Ok(Ok(_)) => {
            warn!("Sentinel: {} was killed before it finished", name);
            None
        }
        Ok(Err(e)) => {
            warn!("Sentinel: could not run {}: {}", name, e);
            None
        }
        Err(_) => {
            warn!("Sentinel: {} took longer than {:?} — stopped", name, limit);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(ok: bool, stdout: &str, stderr: &str) -> Output {
        Output {
            ok,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    fn check_for(manager: PackageManager, source: &str) -> Option<Check> {
        for_this_machine(manager).into_iter().find(|c| c.source == source)
    }

    #[test]
    fn os_release_values_are_unquoted() {
        let text = "NAME=\"Debian GNU/Linux\"\nID=debian\nVERSION_CODENAME=bookworm\n";
        assert_eq!(os_release(text, "ID").as_deref(), Some("debian"));
        assert_eq!(os_release(text, "NAME").as_deref(), Some("Debian GNU/Linux"));
        assert_eq!(os_release(text, "MISSING"), None);
    }

    #[test]
    fn programs_are_found_on_path_or_not_at_all() {
        assert!(find_program("sh").is_some());
        assert!(find_program("no-such-program-mavis").is_none());
    }

    /// A failed tool must give no result, not an empty one: an empty
    /// result reads as "everything was fixed", and the next good run as
    /// a wave of new findings.
    #[test]
    fn a_failed_tool_gives_no_result() {
        // dpkg is on every machine these tests run on.
        let Some(dpkg) = check_for(PackageManager::Dpkg, integrity::SOURCE) else { return };
        assert!((dpkg.parse)(&output(false, "", "dpkg: error: something")).is_none());
        assert_eq!((dpkg.parse)(&output(true, "", "")), Some(Snapshot::new()), "clean is a result");
    }

    #[tokio::test]
    async fn a_tool_that_overruns_is_stopped() {
        let sleep = find_program("sleep").expect("sleep");
        let started = std::time::Instant::now();
        let result = run(&sleep, &["30".to_string()], Duration::from_millis(200)).await;
        assert!(result.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_tool_runs_in_english_and_reports_its_status() {
        let sh = find_program("sh").expect("sh");
        let args = ["-c".to_string(), "echo $LC_ALL; echo oops >&2; exit 3".to_string()];
        let out = run(&sh, &args, Duration::from_secs(10)).await.expect("ran");
        assert!(!out.ok);
        assert_eq!(out.stdout.trim(), "C");
        assert_eq!(out.stderr.trim(), "oops");
    }
}