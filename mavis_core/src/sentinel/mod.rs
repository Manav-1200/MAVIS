// mavis_core/src/sentinel/mod.rs
// Phase 8.5 — System Sentinel.
//
// Phase 8's permission gate audits what MAVIS does. This audits what
// happened to the machine: what the package manager installed, removed
// or replaced, and (in later steps) what changed about who can do what.
//
// The problem it solves is one of awareness, not of detection. A system
// update pulls in a dependency nobody asked for, and it sits there
// unnoticed for days because nothing ever mentions it. MAVIS reads the
// package manager's own transaction log and says so.
//
// Two rules this module holds to:
//
//   1. Severity is static (see change.rs). The LLM may phrase a change
//      more naturally, or raise a severity as a second opinion. It may
//      never lower one, and it never decides severity in the first place.
//
//   2. MAVIS reports facts and leaves the verdict to the user. It does
//      not classify anything as malware. Where a real scanner exists
//      (Defender, ClamAV, XProtect), its findings are reported as that
//      tool's findings. An assistant that implies an all-clear it cannot
//      back up is worse than one that stays quiet.
//
// Off by default, like every other context source, via MAVIS_SENTINEL=1.

pub mod change;
pub mod packages;

/// Whether the sentinel is switched on. Same opt-in shape as the
/// MAVIS_CONTEXT_* sources: reading the machine's package history is
/// still reading something about the user, so it is their call.
pub fn enabled() -> bool {
    matches!(
        std::env::var("MAVIS_SENTINEL").as_deref(),
        Ok("1") | Ok("true")
    )
}