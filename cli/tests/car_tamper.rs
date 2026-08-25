//! Install-path adversarial checks for the UnixFS-basic profile: tampering,
//! traversal, bound evasion, destination aliasing, chunked-File-node lies, and
//! the hardening added over the Gate 2 prototype (canonical varints, duplicate
//! protobuf fields, coalescing guard). Every failure leaves the destination
//! untouched with no staging litter. Fully offline.

use std::collections::BTreeMap;
use std::path::Path;

use intend::car::{
    build_dir, encode_chunked_file, encode_directory, install, preflight, read_car, write_car, Cid,
    PbLink, KUBO_CHUNK_BYTES, MAX_FILES,
};

fn sample_tree() -> (tempfile::TempDir, Cid, BTreeMap<Cid, Vec<u8>>) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("references")).unwrap();
    std::fs::write(src.join("SKILL.md"), b"---\nname: t\n---\n").unwrap();
    std::fs::write(src.join("references").join("a.md"), b"alpha").unwrap();
    std::fs::write(src.join("references").join("b.md"), b"beta").unwrap();
    // One chunked file so every sample exercises the File-node path too.
    let big = vec![0x5au8; KUBO_CHUNK_BYTES + 1000];
    std::fs::write(src.join("references").join("big.bin"), &big).unwrap();
    let mut blocks = BTreeMap::new();
    let (root, _) = build_dir(&src, &mut blocks).unwrap();
    (dir, root, blocks)
}

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
    let (plan, published) = install(root, &b, &out).unwrap();
    assert_eq!(
        published,
        out.canonicalize().unwrap(),
        "canonical publish path returned"
    );
    assert_eq!(plan.files.len(), 4);
    assert_eq!(
        std::fs::read(out.join("references/a.md")).unwrap(),
        b"alpha"
    );
    let big = std::fs::read(out.join("references/big.bin")).unwrap();
    assert_eq!(big.len(), KUBO_CHUNK_BYTES + 1000);
    assert!(big.iter().all(|&b| b == 0x5a), "chunk reassembly is exact");
}

#[test]
fn rejects_tampered_block_at_read() {
    let (_dir, root, blocks) = sample_tree();
    let mut car = write_car(root, &blocks);
    let n = car.len();
    car[n - 1] ^= 0x01;
    let err = read_car(&car).unwrap_err();
    assert!(
        format!("{err:#}").contains("does not hash its contents"),
        "{err:#}"
    );
}

#[test]
fn preflight_rehashes_wrong_bytes_under_valid_key() {
    let (_dir, root, blocks) = sample_tree();
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
}

#[test]
fn rejects_root_substitution_before_any_write() {
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
fn rejects_unreachable_extra_and_missing_child() {
    let (dir, root, blocks) = sample_tree();

    let mut padded = blocks.clone();
    let extra = b"orphan".to_vec();
    padded.insert(Cid::for_block(0x55, &extra), extra);
    let out = dir.path().join("out-pad");
    let err = install(root, &padded, &out).unwrap_err();
    assert!(format!("{err:#}").contains("unreachable"), "{err:#}");
    assert_clean_failure(&out);

    let mut pruned = blocks.clone();
    let leaf = *pruned
        .iter()
        .find(|(c, _)| c.codec == 0x55)
        .map(|(c, _)| c)
        .unwrap();
    pruned.remove(&leaf);
    let out = dir.path().join("out-miss");
    let err = install(root, &pruned, &out).unwrap_err();
    assert!(format!("{err:#}").contains("missing block"), "{err:#}");
    assert_clean_failure(&out);
}

#[test]
fn rejects_occupied_destinations_without_following() {
    let (dir, root, blocks) = sample_tree();
    let out = dir.path().join("occupied");
    std::fs::create_dir(&out).unwrap();
    assert!(install(root, &blocks, &out)
        .unwrap_err()
        .to_string()
        .contains("already exists"));

    let target = dir.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let aliased = dir.path().join("aliased");
    std::os::unix::fs::symlink(&target, &aliased).unwrap();
    assert!(install(root, &blocks, &aliased)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);

    let dangling = dir.path().join("dangling");
    std::os::unix::fs::symlink(dir.path().join("nope"), &dangling).unwrap();
    assert!(install(root, &blocks, &dangling)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
}

#[test]
fn shared_blocks_install_under_every_reference() {
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
    let (plan, _) = install(root, &blocks, &out).unwrap();
    assert_eq!(plan.files.len(), 2);
    assert_eq!(plan.total_bytes, 2 * payload.len() as u64);
    assert_eq!(std::fs::read(out.join("a.md")).unwrap(), payload);
    assert_eq!(std::fs::read(out.join("b.md")).unwrap(), payload);
}

#[test]
fn aggregate_cap_counts_materialized_bytes() {
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
fn file_count_bound_is_effective_within_one_directory() {
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
fn rejects_path_traversal_and_duplicate_names() {
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
    let err = preflight(root, &blocks).unwrap_err();
    assert!(
        format!("{err:#}").contains("duplicate entry name"),
        "{err:#}"
    );
}

#[test]
fn coalescing_guard_rejects_case_and_normalization_collisions() {
    // "Ref.md" vs "ref.md": bytewise distinct, but they land on one file on
    // case-insensitive filesystems (APFS default) — must fail closed.
    let payload = b"data".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let node = encode_directory(&[
        PbLink {
            cid: leaf,
            name: "Ref.md".into(),
            tsize: payload.len() as u64,
        },
        PbLink {
            cid: leaf,
            name: "ref.md".into(),
            tsize: payload.len() as u64,
        },
    ]);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload.clone());
    blocks.insert(root, node);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("folding"), "{err:#}");

    // NFC vs NFD spellings of "é.md" — normalization-insensitive collision.
    // (NFD sorts first bytewise, so list it first: the canonical-order check
    // must NOT be what fires here — the folding guard must.)
    let nfc = "\u{00e9}.md".to_string();
    let nfd = "e\u{0301}.md".to_string();
    assert_ne!(nfc.as_bytes(), nfd.as_bytes());
    let node = encode_directory(&[
        PbLink {
            cid: leaf,
            name: nfd,
            tsize: payload.len() as u64,
        },
        PbLink {
            cid: leaf,
            name: nfc,
            tsize: payload.len() as u64,
        },
    ]);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload);
    blocks.insert(root, node);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("folding"), "{err:#}");
}

// ---------- chunked File-node lies ----------

fn chunked_fixture() -> (Vec<Vec<u8>>, Vec<(Cid, u64)>) {
    let chunks: Vec<Vec<u8>> = vec![vec![1u8; 1000], vec![2u8; 500]];
    let metas: Vec<(Cid, u64)> = chunks
        .iter()
        .map(|c| (Cid::for_block(0x55, c), c.len() as u64))
        .collect();
    (chunks, metas)
}

fn wrap_in_dir(file_node: Vec<u8>, chunks: &[Vec<u8>]) -> (Cid, BTreeMap<Cid, Vec<u8>>) {
    let file_cid = Cid::for_block(0x70, &file_node);
    let dir_node = encode_directory(&[PbLink {
        cid: file_cid,
        name: "file.bin".into(),
        tsize: file_node.len() as u64 + chunks.iter().map(|c| c.len() as u64).sum::<u64>(),
    }]);
    let root = Cid::for_block(0x70, &dir_node);
    let mut blocks = BTreeMap::new();
    for c in chunks {
        blocks.insert(Cid::for_block(0x55, c), c.clone());
    }
    blocks.insert(file_cid, file_node);
    blocks.insert(root, dir_node);
    (root, blocks)
}

#[test]
fn chunked_file_assembles_and_lies_reject() {
    let (chunks, metas) = chunked_fixture();

    // Honest chunked file installs, content reassembled in order.
    let (root, blocks) = wrap_in_dir(encode_chunked_file(&metas), &chunks);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let (plan, _) = install(root, &blocks, &out).unwrap();
    assert_eq!(plan.files.len(), 1);
    let got = std::fs::read(out.join("file.bin")).unwrap();
    assert_eq!(got.len(), 1500);
    assert_eq!(&got[..1000], &[1u8; 1000][..]);
    assert_eq!(&got[1000..], &[2u8; 500][..]);

    // Lying blocksize.
    let mut bad = metas.clone();
    bad[0].1 = 999;
    let (root, blocks) = wrap_in_dir(encode_chunked_file(&bad), &chunks);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("blocksize"), "{err:#}");

    // Named chunk link (hostile producer; decode enforces name emptiness).
    let file_node = build_file_node(&metas, Some("x"), 1500);
    let (root, blocks) = wrap_in_dir(file_node, &chunks);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("carries a name"), "{err:#}");

    // Filesize disagreeing with the chunk sum.
    let file_node = build_file_node(&metas, None, 10);
    let (root, blocks) = wrap_in_dir(file_node, &chunks);
    let err = preflight(root, &blocks).unwrap_err();
    assert!(format!("{err:#}").contains("filesize"), "{err:#}");
}

fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// Hand-assemble a File PBNode so tests can smuggle in lies the honest encoder
/// refuses: an optional chunk-link NAME and an arbitrary declared filesize.
fn build_file_node(metas: &[(Cid, u64)], link_name: Option<&str>, filesize: u64) -> Vec<u8> {
    let mut node = Vec::new();
    for (cid, len) in metas {
        let mut l = Vec::new();
        l.push(0x0a); // PBLink field 1 (Hash), wire 2
        let cid_bytes = cid.to_bytes();
        l.push(cid_bytes.len() as u8);
        l.extend_from_slice(&cid_bytes);
        l.push(0x12); // Name
        match link_name {
            Some(name) => {
                l.push(name.len() as u8);
                l.extend_from_slice(name.as_bytes());
            }
            None => l.push(0x00),
        }
        l.push(0x18); // Tsize
        push_varint(&mut l, *len);
        node.push(0x12); // PBNode field 2 (Links), wire 2
        node.push(l.len() as u8);
        node.extend_from_slice(&l);
    }
    let mut unixfs = vec![0x08, 0x02, 0x18]; // Type=File, then filesize
    push_varint(&mut unixfs, filesize);
    for (_, len) in metas {
        unixfs.push(0x20); // blocksizes
        push_varint(&mut unixfs, *len);
    }
    node.push(0x0a); // PBNode field 1 (Data), wire 2
    node.push(unixfs.len() as u8);
    node.extend_from_slice(&unixfs);
    node
}

#[test]
fn rejects_noncanonical_varint_in_car_framing() {
    let (_dir, root, blocks) = sample_tree();
    let car = write_car(root, &blocks);
    // The first byte is the header-length varint (< 0x80). Re-encode it
    // noncanonically as two bytes: (len | 0x80), 0x00.
    let mut evil = Vec::with_capacity(car.len() + 1);
    evil.push(car[0] | 0x80);
    evil.push(0x00);
    evil.extend_from_slice(&car[1..]);
    let err = read_car(&evil).unwrap_err();
    assert!(
        format!("{err:#}").contains("noncanonical varint"),
        "{err:#}"
    );
}

#[test]
fn rejects_disallowed_codec_and_uppercase_cid_text() {
    let raw = {
        let mut b = vec![0x01, 0x71, 0x12, 0x20];
        b.extend_from_slice(&[0u8; 32]);
        b
    };
    assert!(format!("{:#}", Cid::from_bytes(&raw).unwrap_err()).contains("allowlist"));

    let cid = Cid::for_block(0x70, b"node");
    let s = cid.to_string_canonical();
    assert_eq!(Cid::parse_canonical(&s).unwrap(), cid);
    assert!(Cid::parse_canonical(&s.to_uppercase()).is_err());
}

#[test]
fn rename_noreplace_refuses_raced_empty_directory() {
    // A plain rename() would silently REPLACE an empty destination directory —
    // exactly the TOCTOU window the review named. The no-replace publish must
    // refuse it.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("f"), b"x").unwrap();
    let dst = dir.path().join("dst");
    std::fs::create_dir(&dst).unwrap(); // the "raced" empty dir
    let err = intend::car::rename_noreplace(&src, &dst).unwrap_err();
    assert!(format!("{err:#}").contains("rename_noreplace"), "{err:#}");
    assert!(src.exists(), "source untouched");
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 0, "dst untouched");
}

#[test]
fn filesystem_alias_coalescing_fails_closed_or_installs_distinctly() {
    // Names the NFC+lowercase fold does NOT merge but some filesystems do (APFS
    // folds long-s 'ſ' to 's' and final sigma 'ς' to 'σ'). The exclusive
    // staged-dir/file creation must turn an on-disk merge into a typed failure;
    // on filesystems that keep them distinct, the install must succeed with
    // both entries present. Both outcomes are correct — silent merging is not.
    for (a, b) in [("\u{017f}-dir", "s-dir"), ("\u{03c2}-dir", "\u{03c3}-dir")] {
        let payload = b"data".to_vec();
        let leaf = Cid::for_block(0x55, &payload);
        let sub = encode_directory(&[PbLink {
            cid: leaf,
            name: "f.md".into(),
            tsize: payload.len() as u64,
        }]);
        let sub_cid = Cid::for_block(0x70, &sub);
        let (first, second) = if a.as_bytes() < b.as_bytes() {
            (a, b)
        } else {
            (b, a)
        };
        let top = encode_directory(&[
            PbLink {
                cid: sub_cid,
                name: first.into(),
                tsize: sub.len() as u64,
            },
            PbLink {
                cid: sub_cid,
                name: second.into(),
                tsize: sub.len() as u64,
            },
        ]);
        let root = Cid::for_block(0x70, &top);
        let mut blocks = BTreeMap::new();
        blocks.insert(leaf, payload.clone());
        blocks.insert(sub_cid, sub);
        blocks.insert(root, top);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        match install(root, &blocks, &out) {
            Ok(_) => {
                assert!(out.join(first).join("f.md").exists());
                assert!(out.join(second).join("f.md").exists());
                assert_eq!(
                    std::fs::read_dir(&out).unwrap().count(),
                    2,
                    "both dirs must be genuinely distinct on this filesystem"
                );
            }
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("collides") || msg.contains("folding"),
                    "unexpected failure kind for {a:?}/{b:?}: {msg}"
                );
                assert_clean_failure(&out);
            }
        }
    }
}

#[test]
fn accepts_unsorted_directory_links_and_either_pbnode_order() {
    // Per the UnixFS/dag-pb specs the DECODER accepts any directory link
    // ordering and either PBNode field order; duplicates still reject.
    let payload = b"data".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    // Unsorted links.
    let node = encode_directory(&[
        PbLink {
            cid: leaf,
            name: "b.md".into(),
            tsize: payload.len() as u64,
        },
        PbLink {
            cid: leaf,
            name: "a.md".into(),
            tsize: payload.len() as u64,
        },
    ]);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload.clone());
    blocks.insert(root, node);
    let plan = preflight(root, &blocks).unwrap();
    assert_eq!(plan.files.len(), 2);

    // Data field BEFORE Links (historic order) — hand-assemble.
    let mut link = Vec::new();
    link.push(0x0a);
    let cid_bytes = leaf.to_bytes();
    link.push(cid_bytes.len() as u8);
    link.extend_from_slice(&cid_bytes);
    link.extend_from_slice(&[0x12, 0x04]);
    link.extend_from_slice(b"f.md");
    link.push(0x18);
    link.push(payload.len() as u8);
    let mut node = vec![0x0a, 0x02, 0x08, 0x01]; // Data first: Type=Directory
    node.push(0x12);
    node.push(link.len() as u8);
    node.extend_from_slice(&link);
    let root = Cid::for_block(0x70, &node);
    let mut blocks = BTreeMap::new();
    blocks.insert(leaf, payload);
    blocks.insert(root, node);
    let plan = preflight(root, &blocks).unwrap();
    assert_eq!(plan.files.len(), 1, "Data-before-Links accepted");
}

#[test]
fn rejects_pblink_fields_out_of_order_and_accepts_absent_name() {
    use intend::car::decode_pbnode;
    // Link with Name (2) BEFORE Hash (1): must reject.
    let payload = b"data".to_vec();
    let leaf = Cid::for_block(0x55, &payload);
    let mut link = Vec::new();
    link.extend_from_slice(&[0x12, 0x01, b'x']); // Name first
    link.push(0x0a); // then Hash
    let cid_bytes = leaf.to_bytes();
    link.push(cid_bytes.len() as u8);
    link.extend_from_slice(&cid_bytes);
    let mut node = Vec::new();
    node.push(0x12);
    node.push(link.len() as u8);
    node.extend_from_slice(&link);
    node.extend_from_slice(&[0x0a, 0x02, 0x08, 0x01]); // Data: Type=Directory
    let err = decode_pbnode(&node).unwrap_err();
    assert!(format!("{err:#}").contains("out of order"), "{err:#}");

    // Link with NO Name field at all: decodes with the empty name (current
    // dag-pb practice) — used by File chunk links.
    let mut link = Vec::new();
    link.push(0x0a);
    link.push(cid_bytes.len() as u8);
    link.extend_from_slice(&cid_bytes);
    link.push(0x18);
    link.push(payload.len() as u8);
    let mut node = Vec::new();
    node.push(0x12);
    node.push(link.len() as u8);
    node.extend_from_slice(&link);
    node.extend_from_slice(&[0x0a, 0x02, 0x08, 0x01]);
    let pb = decode_pbnode(&node).unwrap();
    assert_eq!(pb.links.len(), 1);
    assert_eq!(pb.links[0].name, "", "absent Name reads as empty");
}

#[test]
fn read_car_enforces_block_count_bound() {
    use intend::car::MAX_CAR_BLOCKS;
    // Distinct 3-byte raw blocks, one over the bound. ~2.6 MB CAR, in memory.
    let mut blocks = BTreeMap::new();
    let mut i: u32 = 0;
    while blocks.len() <= MAX_CAR_BLOCKS {
        let data = i.to_be_bytes()[1..].to_vec();
        blocks.insert(Cid::for_block(0x55, &data), data);
        i += 1;
    }
    let root = *blocks.keys().next().unwrap();
    let car = write_car(root, &blocks);
    let err = read_car(&car).unwrap_err();
    assert!(format!("{err:#}").contains("block bound"), "{err:#}");
}

#[test]
fn scope_lock_is_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.lock");
    let held = intend::store::ScopeLock::acquire(&path).unwrap();
    assert!(
        intend::store::ScopeLock::try_acquire(&path)
            .unwrap()
            .is_none(),
        "second acquisition must be refused while held"
    );
    drop(held);
    assert!(intend::store::ScopeLock::try_acquire(&path)
        .unwrap()
        .is_some());
}

#[test]
fn duplicate_unixfs_metadata_fields_reject_while_single_ones_are_ignored() {
    // Hand-crafted PBNode whose UnixFS Data carries metadata fields: mode is
    // field 7 varint (key 0x38), mtime is field 8 embedded message (key 0x42).
    let craft = |unixfs: &[u8]| {
        let mut node = Vec::new();
        node.push(0x0a); // PBNode field 1 (Data), wire 2
        node.push(unixfs.len() as u8);
        node.extend_from_slice(unixfs);
        node
    };
    // Type=Directory + ONE mode (0o755) + ONE mtime: accepted, values ignored
    // (mode read-and-dropped; the mtime UnixTime message is length-skipped).
    let single = craft(&[0x08, 0x01, 0x38, 0xed, 0x03, 0x42, 0x02, 0x08, 0x01]);
    intend::car::decode_pbnode(&single).unwrap();
    // DUPLICATE mode: rejected.
    let dup_mode = craft(&[0x08, 0x01, 0x38, 0x01, 0x38, 0x01]);
    let err = format!("{:#}", intend::car::decode_pbnode(&dup_mode).unwrap_err());
    assert!(err.contains("duplicate UnixFS mode"), "{err}");
    // DUPLICATE mtime: rejected.
    let dup_mtime = craft(&[0x08, 0x01, 0x42, 0x00, 0x42, 0x00]);
    let err = format!("{:#}", intend::car::decode_pbnode(&dup_mtime).unwrap_err());
    assert!(err.contains("duplicate UnixFS mtime"), "{err}");
}
