//! The V1 six-column descriptor schema (RFC 0001 §5), RLP-encoded, plus a
//! deterministic synthetic-entry generator. The bench dogfoods the production
//! encoding: itemID = keccak256(RLP list of six UTF-8 strings).

use alloy::primitives::{keccak256, Bytes, B256};
use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq)]
pub struct Descriptor {
    pub name: String,
    pub description: String,
    pub tree_cid: String,
    pub runtimes: String,
    pub origin: String,
    pub reserved: String, // MUST be empty under policy v2
}

impl Descriptor {
    /// RLP list of the six columns in policy order.
    pub fn encode(&self) -> Bytes {
        let cols: [&str; 6] = [
            &self.name,
            &self.description,
            &self.tree_cid,
            &self.runtimes,
            &self.origin,
            &self.reserved,
        ];
        let mut out = Vec::new();
        // RLP: a list of strings.
        alloy::rlp::encode_list::<&str, str>(&cols, &mut out);
        out.into()
    }

    pub fn item_id(&self) -> B256 {
        keccak256(self.encode())
    }

    /// Decode an RLP descriptor back into columns (verification-side check).
    pub fn decode(raw: &[u8]) -> eyre::Result<Self> {
        let cols: Vec<String> = alloy::rlp::decode_exact::<Vec<String>>(raw)
            .map_err(|e| eyre::eyre!("descriptor RLP decode: {e}"))?;
        if cols.len() != 6 {
            eyre::bail!("descriptor has {} columns, expected 6", cols.len());
        }
        let mut it = cols.into_iter();
        Ok(Self {
            name: it.next().unwrap(),
            description: it.next().unwrap(),
            tree_cid: it.next().unwrap(),
            runtimes: it.next().unwrap(),
            origin: it.next().unwrap(),
            reserved: it.next().unwrap(),
        })
    }
}

/// Strict canonical-Tree-CID check per the listing policy: multibase `b`, base32
/// lowercase, no padding, decoding to exactly `0x01 0x70 0x12 0x20` + 32-byte digest
/// (CIDv1, dag-pb, sha2-256/32). `data_encoding` rejects nonzero trailing bits, so a
/// non-canonical final character also fails.
pub fn is_canonical_tree_cid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('b') else {
        return false;
    };
    // 36 bytes => exactly 58 unpadded base32 chars, lowercase alphabet only.
    if rest.len() != 58 || !rest.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')) {
        return false;
    }
    match BASE32_NOPAD.decode(rest.to_uppercase().as_bytes()) {
        Ok(raw) => raw.len() == 36 && raw[..4] == [0x01, 0x70, 0x12, 0x20],
        Err(_) => false,
    }
}

/// Structural descriptor screening — the offline-checkable subset of spec §6 step 7.
/// (Full policy screening — frontmatter byte-match, Origin binding — needs the tree
/// and is the production CLI's scope, not this bench's.)
pub fn screen(d: &Descriptor) -> eyre::Result<()> {
    if !d.reserved.is_empty() {
        eyre::bail!("Reserved column must be the empty string in V1");
    }
    if !is_canonical_tree_cid(&d.tree_cid) {
        eyre::bail!(
            "Tree CID {:?} is not a canonical CIDv1 (base32-lower, dag-pb, sha2-256/32)",
            d.tree_cid
        );
    }
    Ok(())
}

/// Canonical CIDv1 (base32 lowercase, dag-pb codec 0x70, sha2-256 multihash) over
/// arbitrary bytes — well-FORMED per the policy's canonical-CID rule. The referenced
/// content need not exist for snapshot benchmarking.
pub fn canonical_cid_v1(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut bytes = Vec::with_capacity(4 + 32);
    bytes.extend_from_slice(&[0x01, 0x70, 0x12, 0x20]); // v1, dag-pb, sha2-256, len 32
    bytes.extend_from_slice(&digest);
    format!("b{}", BASE32_NOPAD.encode(&bytes).to_lowercase())
}

/// Deterministic synthetic entry `i` — realistic column sizes (short kebab name,
/// one-sentence description, canonical CID, `generic` runtime, empty optionals).
pub fn synthetic(i: u64) -> Descriptor {
    Descriptor {
        name: format!("bench-skill-{i:05}"),
        description: format!(
            "Synthetic benchmark entry {i} for the Intendhub snapshot gate; measures descriptor and proof costs at scale."
        ),
        tree_cid: canonical_cid_v1(format!("intendhub-bench-tree-{i}").as_bytes()),
        runtimes: "generic".into(),
        origin: String::new(),
        reserved: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_roundtrips_and_hashes_deterministically() {
        let d = synthetic(7);
        let raw = d.encode();
        let back = Descriptor::decode(&raw).unwrap();
        assert_eq!(d, back);
        assert_eq!(d.item_id(), synthetic(7).item_id());
        assert_ne!(d.item_id(), synthetic(8).item_id());
    }

    #[test]
    fn synthetic_cid_is_canonical_form() {
        let cid = synthetic(1).tree_cid;
        assert!(cid.starts_with('b'), "multibase base32 prefix");
        assert_eq!(cid, cid.to_lowercase(), "lowercase");
        assert!(!cid.contains('='), "no padding");
        // v1 dag-pb sha2-256 32-byte payload => 36 bytes => 58 base32 chars + prefix.
        assert_eq!(cid.len(), 59, "canonical length");
        assert!(is_canonical_tree_cid(&cid), "strict checker accepts it");
    }

    #[test]
    fn screening_rejects_nonempty_reserved() {
        let mut d = synthetic(1);
        screen(&d).unwrap();
        d.reserved = "v2".into();
        let err = screen(&d).unwrap_err();
        assert!(format!("{err:#}").contains("Reserved"), "{err:#}");
    }

    #[test]
    fn screening_rejects_noncanonical_tree_cid() {
        let good = synthetic(1).tree_cid;
        let bad = [
            good.to_uppercase(),            // uppercase text
            good[1..].to_string(),          // missing multibase prefix
            format!("z{}", &good[1..]),     // base58btc-style prefix
            format!("{good}a"),             // wrong length
            String::new(),                  // empty
            format!("b{}", "a".repeat(58)), // decodes, but wrong prefix bytes
            "ipfs://something".into(),      // URL form
            {
                // CIDv1 raw (0x55) instead of dag-pb — root must be dag-pb.
                let digest = Sha256::digest(b"x");
                let mut bytes = vec![0x01, 0x55, 0x12, 0x20];
                bytes.extend_from_slice(&digest);
                format!("b{}", BASE32_NOPAD.encode(&bytes).to_lowercase())
            },
        ];
        for cid in bad {
            let mut d = synthetic(1);
            d.tree_cid = cid.clone();
            assert!(screen(&d).is_err(), "must reject {cid:?}");
        }
    }
}
