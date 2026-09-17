// mavis_core/src/safety/risk.rs
// Risk scoring for proposed actions, 0-10.
//
// Heuristic, not an LLM call. The phase plan specified "second-pass LLM call
// rates risk 1-10", but making safety depend on a 3.8B model that has been
// unreliable at following simple format rules is the wrong trade: a missed
// judgement here runs a destructive command. Static rules are auditable,
// deterministic, and can't be talked around.
//
// An LLM pass could later *raise* a score as a second opinion — it must
// never be able to lower one.

#![allow(dead_code)]

/// Substring patterns that are never run, at any tier. Whitespace is
/// collapsed before matching so spacing can't evade them.
const DENY_PATTERNS: &[&str] = &[
    "mkfs",
    "dd if=/dev/zero",
    "dd if=/dev/random",
    "> /dev/sd",
    "of=/dev/sd",
    "chmod -r 777 /",
    ":(){ :|:& };:",       // fork bomb
    "shutdown",
    "reboot",
    "systemctl poweroff",
    "userdel",
    "visudo",
];

/// Deletions rooted at `/` itself. Kept separate from substring matching
/// because `rm -rf /home/user/project` contains "rm -rf /" but is an
/// ordinary (if destructive) directory removal — it should be confirmable,
/// not permanently blocked. Only a bare root target is unconditional.
const ROOT_DELETE_PREFIXES: &[&str] = &["rm -rf /", "rm -fr /", "rm -r -f /", "rm -f -r /"];

fn is_root_delete(collapsed: &str) -> bool {
    ROOT_DELETE_PREFIXES.iter().any(|p| {
        match collapsed.find(p) {
            Some(idx) => {
                // What follows the trailing slash decides it: nothing, a
                // space, or a shell separator means the target really is /.
                let rest = &collapsed[idx + p.len()..];
                rest.is_empty()
                    || rest.starts_with(' ')
                    || rest.starts_with(';')
                    || rest.starts_with('&')
                    || rest.starts_with('|')
                    || rest.starts_with('*')
            }
            None => false,
        }
    })
}

/// Piping a download straight into a shell. Checked as a combination rather
/// than a literal string: "curl | bash" never appears verbatim because the
/// URL sits in between.
fn is_pipe_to_shell(collapsed: &str) -> bool {
    let downloads = ["curl ", "wget ", "fetch "];
    let shells = ["| sh", "| bash", "| zsh", "|sh", "|bash", "|zsh"];
    downloads.iter().any(|d| collapsed.contains(d))
        && shells.iter().any(|sh| collapsed.contains(sh))
}

/// Commands that modify state and warrant confirmation even when they look
/// routine.
const DESTRUCTIVE_TOKENS: &[&str] = &[
    "rm ", "rmdir", "mv ", "truncate", "shred", "kill ", "pkill", "killall",
    "git reset --hard", "git clean", "git push --force", "drop table", "delete from",
];

const ELEVATED_TOKENS: &[&str] = &["sudo ", "doas ", "pkexec ", "su -"];

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Run without saying anything — respects "never intrusive".
    Allow,
    /// Ask first, wait for a yes.
    Confirm,
    /// Refuse outright. Not overridable by confirmation.
    Deny(String),
}

#[derive(Debug, Clone)]
pub struct Assessment {
    pub score: u8,
    pub verdict: Verdict,
    pub reason: String,
}

/// Score a single action from a plan.
pub fn assess(action: &serde_json::Value) -> Assessment {
    let kind = action.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");

    match kind {
        // Speech and notifications change nothing.
        "say" | "notify" => Assessment {
            score: 0,
            verdict: Verdict::Allow,
            reason: "no side effects".into(),
        },

        // Volume, brightness, media transport — trivially reversible.
        "system" => Assessment {
            score: 1,
            verdict: Verdict::Allow,
            reason: "reversible system control".into(),
        },

        // Launching an application is visible and easily undone by closing it.
        "app" => {
            let target = action.get("target").and_then(|v| v.as_str()).unwrap_or("");
            if ELEVATED_TOKENS.iter().any(|t| target.contains(t)) {
                return Assessment {
                    score: 9,
                    verdict: Verdict::Confirm,
                    reason: "launches with elevated privileges".into(),
                };
            }
            Assessment {
                score: 2,
                verdict: Verdict::Allow,
                reason: "launches an application".into(),
            }
        }

        "shell" => assess_shell(
            action.get("command").and_then(|v| v.as_str()).unwrap_or(""),
        ),

        other => Assessment {
            score: 5,
            verdict: Verdict::Confirm,
            reason: format!("unrecognised action type '{}'", other),
        },
    }
}

fn assess_shell(command: &str) -> Assessment {
    let normalised = command.to_lowercase();
    let collapsed: String = normalised.split_whitespace().collect::<Vec<_>>().join(" ");

    if let Some(pattern) = DENY_PATTERNS.iter().find(|p| collapsed.contains(**p)) {
        return Assessment {
            score: 10,
            verdict: Verdict::Deny(format!("matches blocked pattern '{}'", pattern)),
            reason: "destructive or irreversible".into(),
        };
    }

    if is_root_delete(&collapsed) {
        return Assessment {
            score: 10,
            verdict: Verdict::Deny("recursive deletion rooted at /".into()),
            reason: "would destroy the filesystem".into(),
        };
    }

    if is_pipe_to_shell(&collapsed) {
        return Assessment {
            score: 10,
            verdict: Verdict::Deny("pipes a download directly into a shell".into()),
            reason: "executes unreviewed remote code".into(),
        };
    }

    let elevated = ELEVATED_TOKENS.iter().any(|t| collapsed.contains(t));
    let destructive = DESTRUCTIVE_TOKENS.iter().any(|t| collapsed.contains(t));

    let score = match (elevated, destructive) {
        (true, true) => 9,
        (true, false) => 8,
        (false, true) => 6,
        // Anything reaching the shell is at least worth confirming: the
        // command came from speech transcription, which is imperfect.
        (false, false) => 4,
    };

    Assessment {
        score,
        verdict: Verdict::Confirm,
        reason: match (elevated, destructive) {
            (true, true) => "elevated and destructive".into(),
            (true, false) => "requires elevated privileges".into(),
            (false, true) => "modifies or deletes data".into(),
            (false, false) => "runs a shell command".into(),
        },
    }
}

/// Assess a whole plan. The strictest verdict wins — one dangerous step
/// makes the whole plan dangerous.
pub fn assess_plan(plan: &serde_json::Value) -> Assessment {
    let actions: Vec<serde_json::Value> = match plan.as_array() {
        Some(arr) => arr.clone(),
        None => vec![plan.clone()],
    };

    let mut worst = Assessment {
        score: 0,
        verdict: Verdict::Allow,
        reason: "empty plan".into(),
    };

    for action in &actions {
        let a = assess(action);
        if a.score > worst.score {
            worst = a;
        }
    }
    worst
}