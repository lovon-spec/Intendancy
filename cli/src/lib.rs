//! `intend` — Intendancy consumer library: verified registry snapshots, exact
//! skill installation, lockfile auditing (verified-snapshot spec v0.2).
//! Pre-1.0: header-quorum anchoring only, JSON debug transport, bounded
//! UnixFS-basic install profile (kubo-vector-proven).

pub mod anchor;
pub mod car;
pub mod chain;
pub mod fetch;
pub mod lockfile;
pub mod policy;
pub mod profile;
pub mod resolve;
pub mod schema;
pub mod snapshot;
pub mod store;
pub mod transport;

/// TEST-ONLY crash injection: hard-aborts the process (no unwinding, no
/// destructors — a real crash) when the `INTEND_FAILPOINT` environment
/// variable equals `name`; a no-op otherwise. The install-transaction crash
/// suite spawns child test processes with this set to kill them at exact
/// transaction boundaries (journal write, staged extraction, rename-before-
/// parent-fsync, finalizing save). Compiled out whenever debug assertions
/// are OFF — the standard `release` profile; a custom release profile that
/// re-enables debug-assertions would retain it (round-5/6) — so production
/// binaries built with the stock profile carry no abort hook.
#[cfg(debug_assertions)]
pub fn failpoint(name: &str) {
    if std::env::var("INTEND_FAILPOINT").as_deref() == Ok(name) {
        std::process::abort();
    }
}

/// Release builds: crash injection does not exist.
#[cfg(not(debug_assertions))]
pub fn failpoint(_name: &str) {}
