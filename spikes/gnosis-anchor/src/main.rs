//! CLI for the Gnosis finalized-state anchor spike. Non-production.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::B256;
use clap::Parser;
use eyre::{bail, Result};
use gnosis_anchor_spike::beacon::Source;
use gnosis_anchor_spike::consensus::TimeSource;
use gnosis_anchor_spike::pipeline::{self, CaptureMeta, PipelineConfig};
use gnosis_anchor_spike::profile;

/// Verify a finalized Gnosis execution state root from an explicit weak-subjectivity
/// checkpoint, then verify Classic GTCR account/storage proofs against it.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Trusted beacon block root to bootstrap from (MANDATORY; no automatic checkpoint
    /// download, community fallback, or silent default exists).
    #[arg(long)]
    checkpoint: B256,

    /// Beacon API base URL (untrusted data source). Required unless --offline-dir.
    #[arg(long)]
    consensus_rpc: Option<String>,

    /// Execution JSON-RPC URL serving eth_getProof (untrusted data source; may differ
    /// from the consensus host — free providers differ in proof retention depth).
    #[arg(long)]
    execution_rpc: Option<String>,

    /// Replay from captured fixtures instead of the network (deterministic).
    #[arg(long, conflicts_with_all = ["consensus_rpc", "execution_rpc"])]
    offline_dir: Option<PathBuf>,

    /// While live: also write every raw response into this directory as fixtures.
    /// Live-only — offline replay must never write over its own inputs or state.
    #[arg(long, conflicts_with = "offline_dir")]
    capture_dir: Option<PathBuf>,

    /// Local fixture profile (target address, pinned codehash, slots).
    #[arg(long, default_value = "gnosis-classic-gtcr-instance3")]
    fixture_profile: String,

    /// Maximum accepted checkpoint age in seconds.
    #[arg(long, default_value_t = 172_800)]
    max_checkpoint_age: u64,

    /// Persist per-registry finalized-anchor high-water marks here.
    /// REQUIRED in live mode: without persistence, rollback protection is ephemeral
    /// and would report a security state that is not actually kept.
    #[arg(long)]
    state_file: Option<PathBuf>,

    /// Explicit operator recovery: accept an anchor older than (or conflicting with)
    /// the stored high-water mark.
    #[arg(long, default_value_t = false)]
    allow_rollback: bool,

    /// Override wall-clock time (unix seconds). OFFLINE REPLAY ONLY — refused in live
    /// mode, where it would bypass checkpoint-age and expected-slot checks. Offline
    /// mode defaults to the captured time from meta.json so replays are deterministic.
    #[arg(long)]
    now_unix: Option<u64>,

    /// Emit the machine-readable JSON report only.
    #[arg(long, default_value_t = false)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let profile = profile::by_name(&args.fixture_profile)
        .ok_or_else(|| eyre::eyre!("unknown fixture profile '{}'", args.fixture_profile))?;

    // Live-mode invariants (state file required, no fixed time, capture/state
    // aliasing) are enforced ONCE, with typed errors, at the library boundary in
    // `pipeline::validate` — no duplicate CLI guards.

    let (source, label, meta): (Source, String, Option<CaptureMeta>) =
        if let Some(dir) = &args.offline_dir {
            let meta: Option<CaptureMeta> = std::fs::read(dir.join("meta.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            (
                Source::Offline(dir.clone()),
                format!("offline fixtures ({})", dir.display()),
                meta,
            )
        } else {
            let consensus_url = args
                .consensus_rpc
                .clone()
                .ok_or_else(|| eyre::eyre!("--consensus-rpc required unless --offline-dir"))?;
            let execution_url = args
                .execution_rpc
                .clone()
                .ok_or_else(|| eyre::eyre!("--execution-rpc required unless --offline-dir"))?;
            (
                Source::Live {
                    consensus_url: consensus_url.clone(),
                    execution_url,
                    capture_dir: args.capture_dir.clone(),
                },
                format!("live ({consensus_url})"),
                None,
            )
        };

    // Live mode always reads the trusted local clock, freshly at every verification
    // (5-second Gnosis slots make a frozen `now` race real signature slots). Offline
    // replay pins time for determinism: --now-unix, else the captured time. An
    // explicit --now-unix is ALWAYS honored as fixed time so that using it live
    // produces the typed `LiveFixedTimeForbidden` library error — never a silent
    // ignore.
    let time = if let Some(n) = args.now_unix {
        TimeSource::FixedForReplay(n)
    } else if args.offline_dir.is_some() {
        TimeSource::FixedForReplay(match (args.now_unix, &meta) {
            (Some(n), _) => n,
            (None, Some(m)) => m.captured_at_unix,
            (None, None) => SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        })
    } else {
        TimeSource::LocalClock
    };

    let cfg = PipelineConfig {
        source,
        checkpoint: args.checkpoint,
        profile,
        max_checkpoint_age_secs: args.max_checkpoint_age,
        time,
        state_file: args.state_file.clone(),
        allow_rollback: args.allow_rollback,
        anchor_source_label: label,
    };

    let (report, outcome) = pipeline::run(&cfg);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for s in &report.stages {
            println!(
                "[{}] {}: {}",
                if s.ok { "ok " } else { "FAIL" },
                s.name,
                s.detail
            );
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    match outcome {
        Ok(()) => Ok(()),
        Err(e) => bail!("verification failed: {e:#}"),
    }
}
