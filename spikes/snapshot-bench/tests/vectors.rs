//! Cross-implementation descriptor vectors: the committed fixtures were produced by
//! the repo's REAL frontend encoder (viem toRlp; see tools/gen-descriptor-vectors.mjs
//! and the provenance header in the fixture) — the Rust codec must byte-match them in
//! both directions and agree on itemIDs. Classic stores opaque bytes, so THIS is the
//! compatibility evidence factory submission alone cannot give.

use std::path::PathBuf;

use alloy::primitives::keccak256;
use serde::Deserialize;
use snapshot_bench::schema::{screen, Descriptor};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VectorFile {
    viem_version: String,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vector {
    label: String,
    source: String,
    columns: [String; 6],
    rlp_hex: String,
    item_id_hex: String,
}

fn load() -> VectorFile {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/descriptor-vectors.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn rust_encoder_matches_independent_vectors() {
    let file = load();
    assert_eq!(file.viem_version, "2.47.6", "provenance pin");
    assert!(
        file.vectors.iter().any(|v| v.source == "frontend-encoder"),
        "at least one vector must come from the real frontend encoder"
    );
    for v in &file.vectors {
        let [name, description, tree_cid, runtimes, origin, reserved] = v.columns.clone();
        let d = Descriptor {
            name,
            description,
            tree_cid,
            runtimes,
            origin,
            reserved,
        };
        let expected_rlp = unhex(&v.rlp_hex);
        assert_eq!(
            d.encode().to_vec(),
            expected_rlp,
            "encode mismatch on {} ({})",
            v.label,
            v.source
        );
        let back = Descriptor::decode(&expected_rlp)
            .unwrap_or_else(|e| panic!("decode failed on {}: {e:#}", v.label));
        assert_eq!(back, d, "decode mismatch on {}", v.label);
        assert_eq!(
            keccak256(&expected_rlp).to_string(),
            v.item_id_hex,
            "itemID mismatch on {}",
            v.label
        );
    }
}

#[test]
fn policy_screening_partitions_the_vectors() {
    // The reserved-nonempty vector round-trips at the CODEC layer but must be
    // rejected by the step-7 POLICY screening; realistic vectors must pass it.
    let file = load();
    let by_label = |l: &str| {
        file.vectors
            .iter()
            .find(|v| v.label == l)
            .unwrap_or_else(|| panic!("vector {l} missing"))
    };
    let realistic = by_label("realistic-skill");
    let d = Descriptor::decode(&unhex(&realistic.rlp_hex)).unwrap();
    screen(&d).expect("realistic vector passes screening");

    let reserved = by_label("reserved-nonempty-encoding-only");
    let d = Descriptor::decode(&unhex(&reserved.rlp_hex)).unwrap();
    let err = screen(&d).unwrap_err();
    assert!(format!("{err:#}").contains("Reserved"), "{err:#}");

    let empty = by_label("all-empty");
    let d = Descriptor::decode(&unhex(&empty.rlp_hex)).unwrap();
    assert!(screen(&d).is_err(), "empty Tree CID must fail screening");
}

#[test]
fn rejects_wrong_column_count_and_trailing_bytes() {
    // Five columns.
    let five: [&str; 5] = ["a", "b", "c", "d", "e"];
    let mut out = Vec::new();
    alloy::rlp::encode_list::<&str, str>(&five, &mut out);
    assert!(Descriptor::decode(&out).is_err(), "5 columns must fail");
    // Seven columns.
    let seven: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];
    let mut out = Vec::new();
    alloy::rlp::encode_list::<&str, str>(&seven, &mut out);
    assert!(Descriptor::decode(&out).is_err(), "7 columns must fail");
    // Valid list with trailing garbage.
    let mut ok = snapshot_bench::schema::synthetic(1).encode().to_vec();
    ok.push(0x00);
    assert!(
        Descriptor::decode(&ok).is_err(),
        "trailing bytes must fail (decode_exact)"
    );
}
