//! Install-path adversarial checks: tampered blocks, path traversal, incomplete or
//! padded DAGs, root substitution, destination aliasing, and bound evasion must all
//! fail closed — and every failure must leave the destination untouched with no
//! staging litter. Legitimate shared blocks must install. Fully offline.

use std::collections::BTreeMap;
use std::path::Path;

use snapshot_bench::car::{
    build_dir, encode_directory, install, preflight, read_car, write_car, Cid, PbLink, MAX_FILES,
};

fn sample_tree() -> (tempfile::TempDir, Cid, BTreeMap<Cid, Vec<u8>>) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("references")).unwrap();
    std::fs::write(src.join("SKILL.md"), b"---\nname: t\n---\n").unwrap();
    std::fs::write(src.join("references").join("a.md"), b"alpha").unwrap();
    std::fs::write(src.join("references").join("b.md"), b"beta").unwrap();
    let mut blocks = BTreeMap::new();
    let (root, _) = build_dir(&src, &mut blocks).unwrap();
    (dir, root, blocks)
}

/// A failed install must leave the destination absent and no `.intend-stage-*`
/// staging directory behind in its parent.
fn assert_clean_failure(dest: &Path) {
    assert!(
        std::fs::symlink_metadata(dest).is_err(),
        "destination must not exist after a failed install"
    );
    let parent = dest.parent().unwrap();
    for entry in std::fs::read_dir(parent).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with(".intend-stage-"),
            "staging litter left behind: {name:?}"
        );
    }
}

#[test]
fn round_trip_installs_byte_identical() {
    let (dir, root, blocks) = sample_tree();
    let car = write_car(root, &blocks);
    let (claimed, b) = read_car(&car).unwrap();
    assert_eq!(claimed, root);
    let out = dir.path().join("out");
    let plan = install(root, &b, &out).unwrap();
    let files = plan.locked_files();
    assert_eq!(files.len(), 3);
    assert_eq!(
        std::fs::read(out.join("references/a.md")).unwrap(),
        b"alpha"
    );
}

#[test]
fn rejects_tampered_block_at_read() {
    let (_dir, root, blocks) = sample_tree();
    let mut car = write_car(root, &blocks);
    let n = car.len();
    car[n - 1] ^= 0x01; // flip a byte inside the last block's payload
    let err = read_car(&car).unwrap_err();
    assert!(
        format!("{err:#}").contains("does not hash its contents"),
        "{err:#}"
    );
}

#[test]
fn rejects_root_substitution_before_any_write() {
    // The CAR is intact but the EXTERNAL expected CID names a different tree — the
    // install must refuse without touching the filesystem.
    let (dir, root, blocks) = sample_tree();
    let mut other = root;
    other.digest[0] ^= 0xff;
    let out = dir.path().join("out-sub");
    let err = install(other, &blocks, &out).unwrap_err();
    assert!(
        format!("{err:#}").contains("does not contain the expected root"),
        "{err:#}"
    );
    assert_clean_failure(&out);
}

#[test]
fn rejects_unreachable_extra_block() {
    let (dir, root, mut blocks) = sample_tree();
    let extra = b"orphan".to_vec();
    blocks.insert(Cid::for_block(0x55, &extra), extra);
    let out = dir.path().join("out2");
    let err = install(root, &blocks, &out).unwrap_err();
    assert!(
        format!("{err:#}").contains("unreachable from the root"),
        "{err:#}"
    );
    assert_clean_failure(&out);
}

#[test]
fn rejects_missing_child_block_with_clean_failure() {
    let (dir, root, mut blocks) = sample_tree();
    // Remove one raw leaf.
    let leaf = *blocks
        .iter()
        .find(|(c, _)| c.codec == 0x55)
        .map(|(c, _)| c)
        .unwrap();
    blocks.remove(&leaf);
    let out = dir.path().join("out3");
    let err = install(root, &blocks, &out).unwrap_err();
    assert!(format!("{err:#}").contains("missing block"), "{err:#}");
    assert_clean_failure(&out);
}

#[test]
fn preflight_rehashes_blocks_wrong_bytes_under_valid_key() {
    // `preflight`/`install` are public and take an arbitrary CID→bytes map — the
    // map key is not evidence. Corrupt bytes under a VALID key (bypassing
    // `read_car`, which hashes at read time) must be rejected by preflight's own
    // rehash, for a raw leaf and for the dag-pb root alike.
    let (_dir, root, blocks) = sample_tree();
    // Leaf corruption.
    let mut tampered = blocks.clone();
    let leaf = *tampered
        .iter()
        .find(|(c, _)| c.codec == 0x55)
        .map(|(c, _)| c)
        .unwrap();
    tampered.insert(leaf, b"evil".to_vec());
    let err = preflight(root, &tampered).unwrap_err();
    assert!(
        format!("{err:#}").contains("does not hash its contents"),
        "{err:#}"
    );
    // Root-node corruption.
    let mut tampered = blocks.clone();
    let mut raw = tampered[&root].clone();
    raw[0] ^= 0x01;
    tampered.insert(root, raw);
    let err = preflight(root, &tampered).unwrap_err();
    assert!(
        format!("{err:#}").contains("does not hash its contents"),
        "{err:#}"
    );
}

#[test]
fn rejects_existing_destination_dir() {
    let (dir, root, blocks) = sample_tree();
    let out = dir.path().join("occupied");
    std::fs::create_dir(&out).unwrap();
    let err = install(root, &blocks, &out).unwrap_err();
    assert!(format!("{err:#}").contains("already exists"), "{err:#}");
}

#[test]
fn rejects_destination_symlink_without_following() {
    let (dir, root, blocks) = sample_tree();
    let target = dir.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let out = dir.path().join("aliased");
    std::os::unix::fs::symlink(&target, &out).unwrap();
    let err = install(root, &blocks, &out).unwrap_err();
    assert!(format!("{err:#}").contains("already exists"), "{err:#}");
    // The symlink target must not have been written through.
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn rejects_dangling_destination_symlink() {
    // Path::exists() would say "nothing here" for a dangling symlink; the installer
    // must still refuse (Gate 1 round-3 lesson).
    let (dir, root, blocks) = sample_tree();
    let out = dir.path().join("dangling");
    std::os::unix::fs::symlink(dir.path().join("nope"), &out).unwrap();
    let err = install(root, &blocks, &out).unwrap_err();
    assert!(format!("{err:#}").contains("already exists"), "{err:#}");
}

#[test]
fn shared_raw_block_installs_under_both_names() {
    // Two files with identical content share one raw block — one CAR block, two
    // links. This is legitimate UnixFS structure and must install, with the block
    // materialized under BOTH paths.
    let dir = tempfile::tempdir().unwrap();
    let payload = b"shared-bytes".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let node = encode_directory(&[
        PbLink {
            cid: leaf,
            name: "a.md".into(),
            tsize: payload.len() as u64,
        },
        PbLink {
            cid: leaf,
            name: "b.md".into(),
            tsize: payload.len() as u64,
        },
    ]);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload.clone());
    blocks.insert(root, node);
    let out = dir.path().join("out");
    let plan = install(root, &blocks, &out).unwrap();
    assert_eq!(plan.files.len(), 2);
    assert_eq!(plan.total_bytes, 2 * payload.len() as u64);
    assert_eq!(std::fs::read(out.join("a.md")).unwrap(), payload);
    assert_eq!(std::fs::read(out.join("b.md")).unwrap(), payload);
}

#[test]
fn shared_subdirectory_installs_under_both_names() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"leafdata".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let sub = encode_directory(&[PbLink {
        cid: leaf,
        name: "f.md".into(),
        tsize: payload.len() as u64,
    }]);
    let sub_cid = Cid::for_block(0x70, &sub);
    let top = encode_directory(&[
        PbLink {
            cid: sub_cid,
            name: "one".into(),
            tsize: sub.len() as u64,
        },
        PbLink {
            cid: sub_cid,
            name: "two".into(),
            tsize: sub.len() as u64,
        },
    ]);
    let root = Cid::for_block(0x70, &top);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload.clone());
    blocks.insert(sub_cid, sub);
    blocks.insert(root, top);
    let out = dir.path().join("out");
    let plan = install(root, &blocks, &out).unwrap();
    assert_eq!(plan.files.len(), 2);
    assert_eq!(std::fs::read(out.join("one/f.md")).unwrap(), payload);
    assert_eq!(std::fs::read(out.join("two/f.md")).unwrap(), payload);
}

#[test]
fn file_count_bound_is_effective_within_one_directory() {
    // MAX_FILES+1 links to ONE shared block inside a single directory node — the old
    // per-directory check never fired here; the per-push bound must.
    let payload = b"x".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let links: Vec<PbLink> = (0..=MAX_FILES)
        .map(|i| PbLink {
            cid: leaf,
            name: format!("f{i:05}"),
            tsize: 1,
        })
        .collect();
    let node = encode_directory(&links);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload);
    blocks.insert(root, node);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("file bound"), "{err:#}");
}

#[test]
fn aggregate_policy_cap_counts_materialized_bytes() {
    // One 1-MiB block shared by three links materializes 3 MiB — past the 2 MiB
    // policy cap even though the CAR itself holds only 1 MiB of payload.
    let payload = vec![0xabu8; 1024 * 1024];
    let leaf = Cid::for_block(0x55, &payload);
    let links: Vec<PbLink> = (0..3)
        .map(|i| PbLink {
            cid: leaf,
            name: format!("big-{i}.bin"),
            tsize: payload.len() as u64,
        })
        .collect();
    let node = encode_directory(&links);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload);
    blocks.insert(root, node);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("policy cap"), "{err:#}");
}

#[test]
fn rejects_path_traversal_names() {
    for evil in ["..", ".", "a/b", "a\\b", "", "x\0y"] {
        let dir = tempfile::tempdir().unwrap();
        let payload = b"data".to_vec();
        let leaf = Cid::for_block(0x55, &payload);
        let node = encode_directory(&[PbLink {
            cid: leaf,
            name: evil.to_string(),
            tsize: payload.len() as u64,
        }]);
        let root = Cid::for_block(0x70, &node);
        let mut blocks = BTreeMap::new();
        blocks.insert(leaf, payload);
        blocks.insert(root, node);
        let out = dir.path().join("out");
        let err = install(root, &blocks, &out).unwrap_err();
        assert!(
            format!("{err:#}").contains("unsafe path component"),
            "name {evil:?}: {err:#}"
        );
        assert_clean_failure(&out);
    }
}

#[test]
fn rejects_duplicate_entry_names() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"data".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let link = PbLink {
        cid: leaf,
        name: "same.md".into(),
        tsize: payload.len() as u64,
    };
    let node = encode_directory(&[link.clone(), link]);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload);
    blocks.insert(root, node);
    let err = install(root, &blocks, &dir.path().join("out")).unwrap_err();
    assert!(
        format!("{err:#}").contains("duplicate entry name"),
        "{err:#}"
    );
}

#[test]
fn rejects_disallowed_cid_codec() {
    // dag-cbor (0x71) is outside the raw/dag-pb allowlist and must fail at CID parse.
    let raw = {
        let mut b = vec![0x01, 0x71, 0x12, 0x20];
        b.extend_from_slice(&[0u8; 32]);
        b
    };
    let err = Cid::from_bytes(&raw).unwrap_err();
    assert!(format!("{err:#}").contains("allowlist"), "{err:#}");
}

#[test]
fn canonical_cid_text_round_trips() {
    let cid = Cid::for_block(0x70, b"node");
    let s = cid.to_string_canonical();
    assert_eq!(Cid::parse_canonical(&s).unwrap(), cid);
    assert!(
        Cid::parse_canonical(&s.to_uppercase()).is_err(),
        "uppercase rejected"
    );
}
