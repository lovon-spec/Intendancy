//! Kubo interop vectors (spec §11 item 2): the bounded UnixFS-basic profile must
//! interoperate with real kubo output — consumer side (parse kubo's CAR, verify,
//! install byte-identically) AND producer side (our builder reproduces kubo's
//! root CID for the same tree). The fixture CAR was exported by kubo offline
//! (`ipfs add -rQ --cid-version 1`; see fixtures/kubo/PROVENANCE.txt) over a tree
//! that exercises nested directories, an empty file, a ~100 KB file, and two
//! files with identical content (kubo ships the shared raw block ONCE — real
//! shared-block evidence).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use intend::car::{build_dir, install, read_car, Cid};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/kubo")
}

fn expected_root() -> Cid {
    let text = std::fs::read_to_string(fixtures().join("root-cid.txt")).unwrap();
    Cid::parse_canonical(text.trim()).unwrap()
}

fn walk_files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn kubo_car_parses_and_roots_match() {
    let raw = std::fs::read(fixtures().join("tree.car")).unwrap();
    let (root, blocks) = read_car(&raw).unwrap();
    assert_eq!(root, expected_root(), "kubo CAR root CID");
    // 9 files on disk, but the duplicate-content pair shares one raw block and
    // chunky.bin (600 KB) splits into 3 chunks + 1 File node: 7 single-block
    // leaves + 3 chunk leaves + 1 File node + 3 directory nodes = 14 blocks.
    assert_eq!(blocks.len(), 14, "kubo dedups the shared raw block");
}

#[test]
fn kubo_car_installs_byte_identical_to_source() {
    let raw = std::fs::read(fixtures().join("tree.car")).unwrap();
    let (_claimed, blocks) = read_car(&raw).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("out");
    // Expected CID is the EXTERNAL authority (here: the recorded kubo root).
    let (plan, _) = install(expected_root(), &blocks, &dest).unwrap();
    let source = walk_files(&fixtures().join("tree"));
    let installed = walk_files(&dest);
    assert_eq!(installed.len(), source.len(), "same file count");
    assert_eq!(installed, source, "byte-identical extraction of kubo's CAR");
    assert_eq!(plan.files.len(), 9, "9 planned files (shared block twice)");
}

#[test]
fn kubo_metadata_car_parses_and_installs() {
    // The metadata vector: the same tree added with `--preserve-mode
    // --preserve-mtime` under deterministic metadata (TZ=UTC fixed mtime on
    // every node, exec bit on references/large.bin, all other modes
    // normalized — regenerated verbatim by tools/gen-kubo-vectors.sh; see
    // PROVENANCE.txt). Mode AND mtime fields are legal UnixFS: the decoder
    // accepts each at most once and DISCARDS the values (mode is read and
    // dropped; the mtime UnixTime message is LENGTH-SKIPPED, not validated —
    // exec bits are non-semantic by owner decision, times are
    // transport-only), so the tree still installs byte-identically. Metadata
    // changes hashes, so producer-side CID equality is deliberately NOT
    // asserted here.
    let install_started = std::time::SystemTime::now();
    let raw = std::fs::read(fixtures().join("tree-mode.car")).unwrap();
    let (root, blocks) = read_car(&raw).unwrap();
    let recorded = std::fs::read_to_string(fixtures().join("mode-root-cid.txt")).unwrap();
    assert_eq!(root, Cid::parse_canonical(recorded.trim()).unwrap());
    assert_ne!(
        root,
        expected_root(),
        "the metadata vector must actually CARRY metadata (its hashes differ)"
    );
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("out");
    let (_plan, _) = install(root, &blocks, &dest).unwrap();
    let source = walk_files(&fixtures().join("tree"));
    let installed = walk_files(&dest);
    assert_eq!(
        installed, source,
        "metadata-bearing tree installs byte-identically"
    );
    // The discard is INTENTIONAL and observable: the vector's recorded exec
    // bit does not reach the installed file, and the recorded 2026-01-01
    // mtime is not preserved (installed files carry install-time mtimes).
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(dest.join("references").join("large.bin")).unwrap();
    assert_eq!(
        meta.permissions().mode() & 0o111,
        0,
        "executable mode must be discarded on install"
    );
    let slack = std::time::Duration::from_secs(5);
    assert!(
        meta.modified().unwrap() >= install_started - slack,
        "mtime must be the install time, not the vector's recorded mtime"
    );
}

#[test]
fn our_builder_reproduces_kubos_root_cid() {
    // Producer-side interop: building the SAME tree with our encoder must yield
    // kubo's exact root CID — link order, Tsize accounting, UnixFS Data typing,
    // raw-leaf choice, and dag-pb field order all have to agree for this to hold.
    let mut blocks = BTreeMap::new();
    let (root, _tsize) = build_dir(&fixtures().join("tree"), &mut blocks).unwrap();
    assert_eq!(
        root.to_string_canonical(),
        expected_root().to_string_canonical(),
        "root CID must byte-match kubo's for the UnixFS-basic profile"
    );
}
