//! Adversarial suite (brief §Deliverables/3): every case must FAIL CLOSED at the
//! correct stage. Each test copies the good fixtures into a tempdir, applies one
//! mutation, and asserts rejection with a diagnostic that matches the attack.

use std::fs;
use std::path::{Path, PathBuf};

use gnosis_anchor_spike::beacon::Source;
use gnosis_anchor_spike::consensus::TimeSource;
use gnosis_anchor_spike::gnosis;
use gnosis_anchor_spike::pipeline::{run, CaptureMeta, PipelineConfig};
use gnosis_anchor_spike::profile::{self, Profile};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn meta() -> CaptureMeta {
    serde_json::from_slice(&fs::read(fixtures_dir().join("meta.json")).unwrap()).unwrap()
}

/// Copy fixtures to a tempdir, optionally mutating one JSON file.
fn mutated(file: Option<&str>, mutate: impl FnOnce(&mut Value)) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for entry in fs::read_dir(fixtures_dir()).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), tmp.path().join(entry.file_name())).unwrap();
    }
    if let Some(name) = file {
        let path = tmp.path().join(name);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut v);
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    tmp
}

fn run_with(
    dir: &Path,
    profile: Profile,
    max_age: u64,
    state_file: Option<PathBuf>,
    checkpoint_override: Option<alloy::primitives::B256>,
) -> (gnosis_anchor_spike::report::Report, eyre::Result<()>) {
    let m = meta();
    run(&PipelineConfig {
        source: Source::Offline(dir.to_path_buf()),
        checkpoint: checkpoint_override.unwrap_or(m.checkpoint),
        profile,
        max_checkpoint_age_secs: max_age,
        time: TimeSource::FixedForReplay(m.captured_at_unix),
        state_file,
        allow_rollback: false,
        anchor_source_label: "adversarial".into(),
    })
}

// -- library-boundary configuration invariants (review blocker 2) -----------
// These must fail BEFORE any network access or response capture. Validation may create
// the configured output directories to resolve their real filesystem identities; the
// URLs below are unreachable, so reaching transport at all changes the error text.

fn live_config(
    state_file: Option<PathBuf>,
    capture_dir: Option<PathBuf>,
    time: TimeSource,
) -> PipelineConfig {
    let m = meta();
    PipelineConfig {
        source: Source::Live {
            consensus_url: "http://127.0.0.1:1".into(),
            execution_url: "http://127.0.0.1:1".into(),
            capture_dir,
        },
        checkpoint: m.checkpoint,
        profile: profile::canary(),
        max_checkpoint_age_secs: 172_800,
        time,
        state_file,
        allow_rollback: false,
        anchor_source_label: "config-invariant test".into(),
    }
}

#[test]
fn live_without_state_file_is_rejected_by_the_library() {
    let (report, outcome) = run(&live_config(None, None, TimeSource::LocalClock));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(
        msg.contains("--state-file is required in live mode"),
        "{msg}"
    );
    assert_eq!(report.stages[0].name, "config");
    assert!(!report.stages[0].ok);
}

#[test]
fn live_with_fixed_time_is_rejected_by_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let (_r, outcome) = run(&live_config(
        Some(tmp.path().join("hw.json")),
        None,
        TimeSource::FixedForReplay(1_787_000_000),
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(
        msg.contains("fixed replay time is forbidden in live mode"),
        "{msg}"
    );
}

#[cfg(unix)]
#[test]
fn live_without_capture_rejects_dangling_state_leaf_symlink() {
    // Rollback state must not silently reset when a symlink target or mount vanishes.
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("unavailable").join("highwater.json");
    let link = tmp.path().join("state.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let (report, outcome) = run(&live_config(
        Some(link.clone()),
        None,
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("state-file leaf is a symbolic link"), "{msg}");
    assert_eq!(report.stages[0].name, "config");
    assert!(!report.stages[0].ok);
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    assert!(!target.exists());
}

#[test]
fn state_file_inside_capture_dir_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    // Exact aliasing of the capture meta path — the reproduced blocker-1 scenario.
    let (_r, outcome) = run(&live_config(
        Some(tmp.path().join("meta.json")),
        Some(tmp.path().to_path_buf()),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

#[test]
fn state_file_dotdot_alias_into_capture_dir_is_rejected() {
    // Round-3 MEDIUM: /x/other/../capture/meta.json must resolve to the real capture
    // dir even though it does not lexically start_with it.
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    let other = tmp.path().join("other");
    fs::create_dir_all(&capture).unwrap();
    fs::create_dir_all(&other).unwrap();
    let aliased = other.join("..").join("capture").join("meta.json");
    let (_r, outcome) = run(&live_config(
        Some(aliased),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

#[cfg(unix)]
#[test]
fn state_file_symlink_alias_into_capture_dir_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    fs::create_dir_all(&capture).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&capture, &link).unwrap();
    let (_r, outcome) = run(&live_config(
        Some(link.join("meta.json")),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

#[cfg(unix)]
#[test]
fn state_file_leaf_symlink_inside_capture_pointing_outside_is_rejected() {
    // Round-5 bypass: resolving only the leaf target made an inside pathname appear
    // outside. Capture would follow the link before high-water persistence replaced it.
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    let outside = tmp.path().join("outside-state.json");
    fs::create_dir_all(&capture).unwrap();
    fs::write(&outside, r#"{"version":1,"marks":{}}"#).unwrap();
    let state_link = capture.join("meta.json");
    std::os::unix::fs::symlink(&outside, &state_link).unwrap();

    let (report, outcome) = run(&live_config(
        Some(state_link),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("state-file leaf is a symbolic link"), "{msg}");
    assert_eq!(report.stages[0].name, "config");
    assert!(!report.stages[0].ok);
    assert_eq!(
        fs::read_to_string(outside).unwrap(),
        r#"{"version":1,"marks":{}}"#
    );
}

#[cfg(unix)]
#[test]
fn state_file_dangling_relative_symlink_activated_by_capture_is_rejected() {
    // Round-4 lifecycle bug: `Path::exists()` returned false for this link while its
    // target was absent; capture creation then activated the alias after validation.
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink("capture", &link).unwrap();
    assert!(!capture.exists(), "capture target must start absent");

    let (report, outcome) = run(&live_config(
        Some(link.join("meta.json")),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
    assert_eq!(report.stages[0].name, "config");
    assert!(!report.stages[0].ok);
}

#[cfg(unix)]
#[test]
fn state_file_dangling_absolute_symlink_activated_by_capture_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&capture, &link).unwrap();
    assert!(!capture.exists(), "capture target must start absent");

    let (_report, outcome) = run(&live_config(
        Some(link.join("meta.json")),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

#[test]
fn state_file_case_alias_of_capture_dir_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let capture = tmp.path().join("capture");
    fs::create_dir_all(&capture).unwrap();
    if !tmp.path().join("CAPTURE").exists() {
        // A case-sensitive filesystem correctly treats these as distinct directories.
        return;
    }
    let (_r, outcome) = run(&live_config(
        Some(tmp.path().join("CAPTURE").join("meta.json")),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

#[test]
fn state_file_unicode_normalization_alias_activated_by_capture_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();

    // Detect whether this filesystem aliases NFC and NFD spellings, using separate
    // probe names so the actual regression still starts with both destinations absent.
    let probe_nfc = tmp.path().join("probe-\u{00e9}");
    let probe_nfd = tmp.path().join("probe-e\u{0301}");
    fs::create_dir(&probe_nfc).unwrap();
    let aliases = probe_nfd.exists();
    fs::remove_dir(&probe_nfc).unwrap();
    if !aliases {
        return;
    }

    let capture = tmp.path().join("capture-\u{00e9}");
    let state_parent = tmp.path().join("capture-e\u{0301}");
    assert!(!capture.exists() && !state_parent.exists());
    let (_report, outcome) = run(&live_config(
        Some(state_parent.join("meta.json")),
        Some(capture),
        TimeSource::LocalClock,
    ));
    let msg = format!("{:#}", outcome.expect_err("must fail"));
    assert!(msg.contains("inside the capture directory"), "{msg}");
}

fn assert_fails(dir: &tempfile::TempDir, expect_substring: &str) {
    let (report, outcome) = run_with(dir.path(), profile::canary(), 172_800, None, None);
    let err = outcome.expect_err("mutation must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase()
            .contains(&expect_substring.to_lowercase()),
        "expected error containing {expect_substring:?}, got: {msg}\nstages: {:?}",
        report
            .stages
            .iter()
            .map(|s| (s.name, s.ok))
            .collect::<Vec<_>>()
    );
}

// -- consensus-layer attacks ------------------------------------------------

#[test]
fn rejects_wrong_checkpoint_root() {
    let m = meta();
    let mut bad = m.checkpoint;
    bad.0[31] ^= 0xff;
    // The bootstrap fixture for the mutated root does not exist under that name, so
    // point the bad-root request at the good bootstrap body: header hash must mismatch.
    let tmp = mutated(None, |_| {});
    fs::copy(
        tmp.path().join(format!("bootstrap-{}.json", m.checkpoint)),
        tmp.path().join(format!("bootstrap-{bad}.json")),
    )
    .unwrap();
    let (_r, outcome) = run_with(tmp.path(), profile::canary(), 172_800, None, Some(bad));
    let msg = format!("{:#}", outcome.expect_err("wrong checkpoint must fail"));
    assert!(
        msg.contains("bootstrap"),
        "expected bootstrap-stage failure, got: {msg}"
    );
}

#[test]
fn rejects_corrupted_sync_signature() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        let sig = v["data"]["sync_aggregate"]["sync_committee_signature"]
            .as_str()
            .unwrap()
            .to_string();
        let flipped = flip_last_hex_nibble(&sig);
        v["data"]["sync_aggregate"]["sync_committee_signature"] = Value::String(flipped);
    });
    assert_fails(&tmp, "finality update");
}

/// NOTE: zeroing the bits while keeping the (now-mismatching) signature is rejected by
/// UPSTREAM signature verification, before the spike's own ≥2/3 supermajority guard.
/// The guard itself is covered by typed unit tests on
/// `consensus::require_supermajority` (341 fails, 342 passes) — see review test gap.
#[test]
fn rejects_zeroed_participation_bits_at_signature_layer() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        let bits = v["data"]["sync_aggregate"]["sync_committee_bits"]
            .as_str()
            .unwrap()
            .len();
        // all-zero participation of the same encoded length
        v["data"]["sync_aggregate"]["sync_committee_bits"] =
            Value::String(format!("0x{}", "0".repeat(bits - 2)));
    });
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_corrupted_finality_branch() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        let first = v["data"]["finality_branch"][0]
            .as_str()
            .unwrap()
            .to_string();
        v["data"]["finality_branch"][0] = Value::String(flip_last_hex_nibble(&first));
    });
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_corrupted_execution_branch() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        let first = v["data"]["finalized_header"]["execution_branch"][0]
            .as_str()
            .unwrap()
            .to_string();
        v["data"]["finalized_header"]["execution_branch"][0] =
            Value::String(flip_last_hex_nibble(&first));
    });
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_substituted_execution_state_root() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        let sr = v["data"]["finalized_header"]["execution"]["state_root"]
            .as_str()
            .unwrap()
            .to_string();
        v["data"]["finalized_header"]["execution"]["state_root"] =
            Value::String(flip_last_hex_nibble(&sr));
    });
    // The substituted stateRoot breaks the execution-payload proof inside the header.
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_tampered_execution_block_number() {
    // Review fix 4: the authenticated block NUMBER is part of the execution payload
    // header; tampering must break the execution-branch proof.
    let tmp = mutated(Some("finality_update.json"), |v| {
        let n: u64 = v["data"]["finalized_header"]["execution"]["block_number"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        v["data"]["finalized_header"]["execution"]["block_number"] =
            Value::String((n + 1).to_string());
    });
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_tampered_execution_block_hash() {
    // Review fix 4: the authenticated block HASH likewise.
    let tmp = mutated(Some("finality_update.json"), |v| {
        let h = v["data"]["finalized_header"]["execution"]["block_hash"]
            .as_str()
            .unwrap()
            .to_string();
        v["data"]["finalized_header"]["execution"]["block_hash"] =
            Value::String(flip_last_hex_nibble(&h));
    });
    assert_fails(&tmp, "finality update");
}

#[test]
fn rejects_unknown_fork_version_fail_closed() {
    let tmp = mutated(Some("finality_update.json"), |v| {
        v["version"] = Value::String("gloas".into());
    });
    assert_fails(&tmp, "unsupported light-client fork version");
}

#[test]
fn rejects_mainnet_constants_on_gnosis_data() {
    // Decoding + verifying Gnosis light-client data with Ethereum-mainnet constants
    // (32-slot epochs, mainnet fork versions, mainnet genesis root) must fail: the
    // sync-committee period math and signing domain are both wrong.
    use helios_consensus_core::consensus_spec::MainnetConsensusSpec;
    use helios_consensus_core::types::{Bootstrap, LightClientStore};
    use helios_consensus_core::{apply_bootstrap, verify_bootstrap, verify_finality_update};

    let m = meta();
    let raw = fs::read(fixtures_dir().join(format!("bootstrap-{}.json", m.checkpoint))).unwrap();
    let v: Value = serde_json::from_slice(&raw).unwrap();
    let bootstrap: Bootstrap<MainnetConsensusSpec> =
        serde_json::from_value(v["data"].clone()).expect("decode with mainnet bounds");

    // Ethereum mainnet fork parameters (genesis root + schedule) applied to Gnosis data.
    let mainnet_genesis_root: alloy::primitives::B256 =
        "0x4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"
            .parse()
            .unwrap();
    let mut forks = gnosis::forks();
    forks.electra.fork_version = [0x05, 0x00, 0x00, 0x00].into();
    forks.fulu.fork_version = [0x06, 0x00, 0x00, 0x00].into();

    let bootstrap_ok = verify_bootstrap::<MainnetConsensusSpec>(&bootstrap, m.checkpoint, &forks);
    let finality_fails = {
        let mut store: LightClientStore<MainnetConsensusSpec> = LightClientStore::default();
        if bootstrap_ok.is_ok() {
            apply_bootstrap(&mut store, &bootstrap);
        }
        let raw = fs::read(fixtures_dir().join("finality_update.json")).unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        match serde_json::from_value::<
            helios_consensus_core::types::FinalityUpdate<MainnetConsensusSpec>,
        >(v["data"].clone())
        {
            Err(_) => true, // fails at decode: also a valid rejection
            Ok(fu) => verify_finality_update::<MainnetConsensusSpec>(
                &fu,
                gnosis::expected_current_slot(m.captured_at_unix),
                &store,
                mainnet_genesis_root,
                &forks,
            )
            .is_err(),
        }
    };
    assert!(
        bootstrap_ok.is_err() || finality_fails,
        "mainnet constants must not verify Gnosis light-client data"
    );
}

// -- execution-proof attacks ------------------------------------------------

#[test]
fn rejects_malformed_account_proof() {
    let m = meta();
    let tmp = mutated(
        Some(&format!("proof-{}.json", m.execution_block_number)),
        |v| {
            let arr = v["result"]["accountProof"].as_array_mut().unwrap();
            arr.pop();
        },
    );
    assert_fails(&tmp, "account proof");
}

#[test]
fn rejects_wrong_pinned_codehash() {
    let tmp = mutated(None, |_| {});
    let mut p = profile::canary();
    p.runtime_code_hash.0[0] ^= 0xff;
    let (_r, outcome) = run_with(tmp.path(), p, 172_800, None, None);
    let msg = format!("{:#}", outcome.expect_err("codehash mismatch must fail"));
    assert!(msg.contains("code hash mismatch"), "got: {msg}");
}

#[test]
fn rejects_corrupted_storage_value() {
    let m = meta();
    let tmp = mutated(
        Some(&format!("proof-{}.json", m.execution_block_number)),
        |v| {
            v["result"]["storageProof"][0]["value"] = Value::String("0xdeadbeef".into());
        },
    );
    assert_fails(&tmp, "storage proof");
}

// -- freshness and rollback -------------------------------------------------

#[test]
fn rejects_stale_checkpoint() {
    let tmp = mutated(None, |_| {});
    let (_r, outcome) = run_with(tmp.path(), profile::canary(), 1, None, None);
    let msg = format!("{:#}", outcome.expect_err("stale checkpoint must fail"));
    assert!(msg.contains("stale checkpoint"), "got: {msg}");
}

#[test]
fn rejects_anchor_rollback() {
    let m = meta();
    let tmp = mutated(None, |_| {});
    let state = tmp.path().join("hw.json");
    fs::write(
        &state,
        serde_json::json!({ "version": 1, "marks": { format!("100:{}", profile::canary().address.to_string().to_lowercase()): {
            "block_number": m.execution_block_number + 1000,
            "block_hash": m.execution_block_hash,
        }}})
        .to_string(),
    )
    .unwrap();
    let (_r, outcome) = run_with(tmp.path(), profile::canary(), 172_800, Some(state), None);
    let msg = format!("{:#}", outcome.expect_err("rollback must fail"));
    assert!(msg.contains("rollback"), "got: {msg}");
}

#[test]
fn rejects_conflicting_hash_at_same_height() {
    let m = meta();
    let tmp = mutated(None, |_| {});
    let state = tmp.path().join("hw.json");
    let mut other = m.execution_block_hash;
    other.0[0] ^= 0xff;
    fs::write(
        &state,
        serde_json::json!({ "version": 1, "marks": { format!("100:{}", profile::canary().address.to_string().to_lowercase()): {
            "block_number": m.execution_block_number,
            "block_hash": other,
        }}})
        .to_string(),
    )
    .unwrap();
    let (_r, outcome) = run_with(tmp.path(), profile::canary(), 172_800, Some(state), None);
    let msg = format!("{:#}", outcome.expect_err("conflicting hash must fail"));
    assert!(msg.contains("conflicting block hash"), "got: {msg}");
}

// ---------------------------------------------------------------------------

fn flip_last_hex_nibble(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    let last = chars.last_mut().unwrap();
    *last = if *last == '0' { '1' } else { '0' };
    chars.into_iter().collect()
}
