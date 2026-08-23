//! Default test suite: deterministic offline replay of the checked-in fixture set.
//! No network access; `now` is pinned by fixtures/meta.json.

use std::path::PathBuf;

use gnosis_anchor_spike::beacon::Source;
use gnosis_anchor_spike::consensus::TimeSource;
use gnosis_anchor_spike::pipeline::{run, CaptureMeta, PipelineConfig};
use gnosis_anchor_spike::profile;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn meta() -> CaptureMeta {
    let raw = std::fs::read(fixtures_dir().join("meta.json"))
        .expect("fixtures/meta.json missing — regenerate with the live capture command in README");
    serde_json::from_slice(&raw).expect("meta.json parse")
}

fn config(state_file: Option<PathBuf>) -> PipelineConfig {
    let m = meta();
    PipelineConfig {
        source: Source::Offline(fixtures_dir()),
        checkpoint: m.checkpoint,
        profile: profile::canary(),
        max_checkpoint_age_secs: 172_800,
        time: TimeSource::FixedForReplay(m.captured_at_unix),
        state_file,
        allow_rollback: false,
        anchor_source_label: "offline fixtures".into(),
    }
}

#[test]
fn offline_pipeline_verifies_end_to_end() {
    let m = meta();
    let tmp = tempfile::tempdir().unwrap();
    let (report, outcome) = run(&config(Some(tmp.path().join("hw.json"))));
    if let Err(e) = &outcome {
        panic!(
            "pipeline failed: {e:#}\nstages: {:?}",
            report
                .stages
                .iter()
                .map(|s| (s.name, s.ok))
                .collect::<Vec<_>>()
        );
    }

    // Every stage passed.
    assert!(report.stages.iter().all(|s| s.ok), "all stages ok");

    // The anchored execution block matches the capture exactly.
    let exec = report.execution.as_ref().expect("execution anchor");
    assert_eq!(exec.block_number, m.execution_block_number);
    assert_eq!(exec.block_hash, m.execution_block_hash);
    assert_eq!(exec.state_root, m.execution_state_root);

    // Signed advancement actually happened (checkpoint predates the target).
    let fb = report.finalized_beacon.as_ref().expect("finalized beacon");
    assert!(
        fb.slot > fb.bootstrap_slot,
        "finality advanced past bootstrap"
    );
    assert!(
        fb.updates_applied >= 1,
        "at least one sync-committee update verified"
    );
    assert!(
        fb.sync_participation * 3 >= 512 * 2,
        "supermajority participation"
    );

    // The Classic GTCR canary slot decoded as expected.
    let storage = report.storage.as_ref().expect("storage results");
    let length = &storage[0];
    assert_eq!(
        Some(format!("{:#x}", length.value)),
        m.item_list_length,
        "itemList.length matches capture"
    );
    assert!(length.meaning.contains("itemCount"));
}

#[test]
fn offline_replay_is_deterministic() {
    let t1 = tempfile::tempdir().unwrap();
    let t2 = tempfile::tempdir().unwrap();
    let (r1, o1) = run(&config(Some(t1.path().join("hw.json"))));
    let (r2, o2) = run(&config(Some(t2.path().join("hw.json"))));
    assert!(o1.is_ok() && o2.is_ok());
    assert_eq!(
        serde_json::to_string(&r1).unwrap(),
        serde_json::to_string(&r2).unwrap(),
        "byte-identical reports across replays"
    );
}
