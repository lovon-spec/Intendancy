//! Resource-limit tests: every bound in `Limits` must actually reject, including the
//! bounded gzip reader (compressed cap + decoded/bomb cap). Bounds are injectable, so
//! each test shrinks exactly one limit below the healthy fixture's real footprint and
//! asserts the typed rejection. Offline.

use std::io::Write;
use std::path::PathBuf;

use snapshot_bench::chain::SeedManifest;
use snapshot_bench::snapshot::{read_snapshot_bounded, verify, Limits, Snapshot};

fn fixtures() -> (SeedManifest, Snapshot) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let manifest: SeedManifest =
        serde_json::from_slice(&std::fs::read(dir.join("manifest-25.json")).unwrap()).unwrap();
    let snapshot: Snapshot =
        serde_json::from_slice(&std::fs::read(dir.join("snapshot-25.json")).unwrap()).unwrap();
    (manifest, snapshot)
}

fn expect_limit_err(limits: &Limits, needle: &str) {
    let (m, s) = fixtures();
    let err = verify(&s, &m.profile(), limits).expect_err("bound must reject");
    let msg = format!("{err:#}");
    assert!(msg.contains(needle), "expected {needle:?}, got: {msg}");
}

#[test]
fn healthy_fixture_passes_default_limits() {
    let (m, s) = fixtures();
    verify(&s, &m.profile(), &Limits::default()).expect("fixture verifies");
}

#[test]
fn rejects_item_count_over_bound() {
    let limits = Limits {
        max_items: 10,
        ..Limits::default()
    };
    expect_limit_err(&limits, "exceeds bound 10");
}

#[test]
fn rejects_row_count_over_bound() {
    // Isolate the ROW-COUNT branch: itemCount passes the cap while rows.len() does
    // not (the itemCount check would otherwise shadow it).
    let (m, mut s) = fixtures();
    s.item_count = 10;
    let limits = Limits {
        max_items: 20,
        ..Limits::default()
    };
    let err = verify(&s, &m.profile(), &limits).expect_err("bound must reject");
    assert!(format!("{err:#}").contains("row count"), "{err:#}");
}

#[test]
fn rejects_slot_count_over_bound() {
    // Isolate the SLOT-COUNT branch: itemCount and rows pass the cap while the slot
    // proof list (untouched: 51 entries) exceeds 1 + 2*cap = 41. Slot contents are
    // never reached — the count trips first.
    let (m, mut s) = fixtures();
    s.item_count = 10;
    s.rows.truncate(10);
    let limits = Limits {
        max_items: 20,
        ..Limits::default()
    };
    let err = verify(&s, &m.profile(), &limits).expect_err("bound must reject");
    assert!(format!("{err:#}").contains("slot proof count"), "{err:#}");
}

#[test]
fn rejects_oversized_storage_node() {
    // Isolate the STORAGE-node size branch under DEFAULT limits: inflate one store
    // entry past 16 KiB (the size check precedes the hash check, and account nodes
    // are untouched, so the error must name the store node, not the account proof).
    let (m, mut s) = fixtures();
    let key = *s.proofs.nodes.keys().next().unwrap();
    s.proofs.nodes.insert(key, vec![0u8; 16 * 1024 + 1].into());
    let err = verify(&s, &m.profile(), &Limits::default()).expect_err("bound must reject");
    let msg = format!("{err:#}");
    assert!(msg.contains("exceeds 16384 bytes"), "{msg}");
    assert!(!msg.contains("account proof node"), "{msg}");
}

#[test]
fn rejects_long_storage_proof_path() {
    // Isolate the STORAGE-path length branch under DEFAULT limits: extend one slot
    // proof's path past 66 refs (the account path, ~4 nodes, passes untouched; the
    // dummy refs are never resolved — the length check trips first).
    let (m, mut s) = fixtures();
    let sp = s.proofs.slots.last_mut().unwrap();
    let filler = *sp.path.first().unwrap();
    sp.path.extend(std::iter::repeat_n(filler, 70));
    let err = verify(&s, &m.profile(), &Limits::default()).expect_err("bound must reject");
    let msg = format!("{err:#}");
    assert!(msg.contains("proof path length"), "{msg}");
    assert!(!msg.contains("account"), "{msg}");
}

#[test]
fn rejects_node_count_over_bound() {
    let limits = Limits {
        max_nodes: 3,
        ..Limits::default()
    };
    expect_limit_err(&limits, "node count");
}

#[test]
fn rejects_node_store_bytes_over_bound() {
    let limits = Limits {
        max_node_store_bytes: 512,
        ..Limits::default()
    };
    expect_limit_err(&limits, "aggregate bound");
}

#[test]
fn rejects_oversized_node() {
    let limits = Limits {
        max_node_bytes: 16,
        ..Limits::default()
    };
    // With a 16-byte per-node cap, either an account node or a store node trips
    // first depending on iteration order; both carry "exceeds".
    expect_limit_err(&limits, "exceeds 16 bytes");
}

#[test]
fn rejects_long_proof_path() {
    let limits = Limits {
        max_path_nodes: 1,
        ..Limits::default()
    };
    expect_limit_err(&limits, "path length");
}

#[test]
fn account_nodes_are_bounded_too() {
    // Shrink the per-node cap to below the account-proof root node size but keep the
    // store irrelevant by checking the error names the ACCOUNT proof when it trips
    // first (account bounds run before the store sweep).
    let (m, s) = fixtures();
    let smallest_account_node = s.proofs.account.iter().map(|n| n.len()).min().unwrap();
    let limits = Limits {
        max_node_bytes: smallest_account_node.saturating_sub(1),
        ..Limits::default()
    };
    let err = verify(&s, &m.profile(), &limits).expect_err("bound must reject");
    assert!(
        format!("{err:#}").contains("account proof node exceeds"),
        "{err:#}"
    );
}

#[test]
fn gzip_transport_roundtrips_under_limits() {
    let (m, s) = fixtures();
    let raw = serde_json::to_vec(&s).unwrap();
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw).unwrap();
    let gz = enc.finish().unwrap();
    let (parsed, decoded) = read_snapshot_bounded(&gz, &Limits::default()).unwrap();
    assert_eq!(decoded, raw.len() as u64);
    verify(&parsed, &m.profile(), &Limits::default()).expect("gzip form verifies");
}

#[test]
fn rejects_compressed_input_over_cap() {
    let (_m, s) = fixtures();
    let raw = serde_json::to_vec(&s).unwrap();
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw).unwrap();
    let gz = enc.finish().unwrap();
    let limits = Limits {
        max_compressed_bytes: 64,
        ..Limits::default()
    };
    let err = read_snapshot_bounded(&gz, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("compressed snapshot"),
        "{err:#}"
    );
}

#[test]
fn rejects_gzip_bomb_at_decoded_cap() {
    // 8 MiB of zeros compresses to ~8 KiB; a 1 MiB decoded cap must stop inflation.
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&vec![0u8; 8 * 1024 * 1024]).unwrap();
    let bomb = enc.finish().unwrap();
    assert!(bomb.len() < 64 * 1024, "bomb should be small compressed");
    let limits = Limits {
        max_decoded_bytes: 1024 * 1024,
        ..Limits::default()
    };
    let err = read_snapshot_bounded(&bomb, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("decoded snapshot exceeds"),
        "{err:#}"
    );
}

#[test]
fn rejects_raw_input_over_decoded_cap() {
    let (_m, s) = fixtures();
    let raw = serde_json::to_vec(&s).unwrap();
    let limits = Limits {
        max_decoded_bytes: 64,
        ..Limits::default()
    };
    let err = read_snapshot_bounded(&raw, &limits).unwrap_err();
    assert!(format!("{err:#}").contains("decoded bound"), "{err:#}");
}

// ---- file-boundary reader: caps enforced BEFORE materializing the input ----
// (`read_snapshot_file_bounded` streams through `Read::take`, so at most cap + 1
// bytes are ever read/allocated no matter how large the on-disk file is.)

#[test]
fn file_boundary_reads_fixture_under_default_limits() {
    let (m, _s) = fixtures();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/snapshot-25.json");
    let (parsed, _) =
        snapshot_bench::snapshot::read_snapshot_file_bounded(&path, &Limits::default()).unwrap();
    verify(&parsed, &m.profile(), &Limits::default()).expect("file path verifies");
}

#[test]
fn file_boundary_rejects_oversized_raw_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.json");
    std::fs::write(&path, vec![b'x'; 8 * 1024]).unwrap();
    let limits = Limits {
        max_decoded_bytes: 1024,
        ..Limits::default()
    };
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&path, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("file exceeds decoded bound"),
        "{err:#}"
    );
}

#[test]
fn file_boundary_zero_cap_rejects_cleanly() {
    // Degenerate injected caps must reject with a typed error, not under-flow:
    // before the saturating fix, cap=0 with a >=2-byte file computed `0 + 1 - 2`
    // (debug panic; release wrap defeating the read bound entirely).
    let dir = tempfile::tempdir().unwrap();
    let raw_path = dir.path().join("two-bytes.json");
    std::fs::write(&raw_path, b"{}").unwrap();
    let limits = Limits {
        max_decoded_bytes: 0,
        ..Limits::default()
    };
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&raw_path, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("exceeds decoded bound 0"),
        "{err:#}"
    );
    // Same for the compressed cap with gzip-magic input.
    let gz_path = dir.path().join("two-bytes.gz");
    std::fs::write(&gz_path, [0x1f, 0x8b, 0x00]).unwrap();
    let limits = Limits {
        max_compressed_bytes: 0,
        ..Limits::default()
    };
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&gz_path, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("compressed snapshot file exceeds bound 0"),
        "{err:#}"
    );
}

#[test]
fn file_boundary_tiny_cap_boundary_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_decoded_bytes: 1,
        ..Limits::default()
    };
    // Two bytes under a 1-byte cap: rejected at the bound.
    let over = dir.path().join("over.json");
    std::fs::write(&over, b"{}").unwrap();
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&over, &limits).unwrap_err();
    assert!(format!("{err:#}").contains("decoded bound 1"), "{err:#}");
    // One byte under a 1-byte cap: passes the bound, fails later at JSON parse.
    let at = dir.path().join("at.json");
    std::fs::write(&at, b"x").unwrap();
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&at, &limits).unwrap_err();
    assert!(format!("{err:#}").contains("snapshot parse"), "{err:#}");
}

#[test]
fn extreme_caps_do_not_overflow() {
    // u64::MAX caps mean "effectively unbounded" and must not overflow anywhere
    // (`cap + 1` sites and the `1 + 2 * max_items` slot bound were the risks).
    let (m, _s) = fixtures();
    let limits = Limits {
        max_compressed_bytes: u64::MAX,
        max_decoded_bytes: u64::MAX,
        max_items: u64::MAX,
        ..Limits::default()
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/snapshot-25.json");
    let (parsed, _) = snapshot_bench::snapshot::read_snapshot_file_bounded(&path, &limits).unwrap();
    verify(&parsed, &m.profile(), &limits).expect("extreme caps verify cleanly");
}

#[test]
fn file_boundary_rejects_oversized_gzip_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.json.gz");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
    enc.write_all(&vec![b'x'; 8 * 1024]).unwrap();
    std::fs::write(&path, enc.finish().unwrap()).unwrap();
    let limits = Limits {
        max_compressed_bytes: 1024,
        ..Limits::default()
    };
    let err = snapshot_bench::snapshot::read_snapshot_file_bounded(&path, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("compressed snapshot file exceeds"),
        "{err:#}"
    );
}

#[test]
fn oversized_account_proof_rejects_at_the_wire_boundary() {
    // PR #3 re-review: "0x" account-proof elements amplify ~14x from JSON
    // bytes to heap; the deserializer itself bounds the collection so the
    // reject happens during parsing, before the flood materializes.
    let (_manifest, snapshot) = fixtures();
    let mut v = serde_json::to_value(&snapshot).unwrap();
    v["proofs"]["account"] =
        serde_json::Value::Array(vec![serde_json::Value::String("0x".into()); 500]);
    let raw = serde_json::to_vec(&v).unwrap();
    let err = format!(
        "{:#}",
        read_snapshot_bounded(&raw, &Limits::default()).unwrap_err()
    );
    assert!(err.contains("account proof exceeds"), "{err}");
    // The honest fixture still round-trips (control).
    let raw = serde_json::to_vec(&snapshot).unwrap();
    read_snapshot_bounded(&raw, &Limits::default()).unwrap();
}
