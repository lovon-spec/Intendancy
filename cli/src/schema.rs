//! The V1 six-column descriptor schema (spec §3.1, FROZEN): RLP list of exactly six
//! UTF-8 strings, `itemID = keccak256(descriptor)`, plus the structural policy
//! screening (spec §6 step 7's offline-checkable subset). Ported from the Gate 2
//! reference implementation; the cross-implementation vectors live in
//! `spikes/snapshot-bench/fixtures/descriptor-vectors.json`.

use alloy::primitives::{keccak256, Bytes, B256};
use data_encoding::BASE32_NOPAD;

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
        alloy::rlp::encode_list::<&str, str>(&cols, &mut out);
        out.into()
    }

    pub fn item_id(&self) -> B256 {
        keccak256(self.encode())
    }

    /// Decode an RLP descriptor back into columns (verification-side check).
    /// Exact-length decoding: trailing bytes reject.
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
/// and runs at install time where applicable.)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Descriptor {
        Descriptor {
            name: "sample-skill".into(),
            description: "A sample.".into(),
            // canonical CIDv1 dag-pb sha2-256 over arbitrary bytes
            tree_cid: {
                use sha2::{Digest, Sha256};
                let digest = Sha256::digest(b"sample");
                let mut bytes = vec![0x01, 0x70, 0x12, 0x20];
                bytes.extend_from_slice(&digest);
                format!("b{}", BASE32_NOPAD.encode(&bytes).to_lowercase())
            },
            runtimes: "generic".into(),
            origin: String::new(),
            reserved: String::new(),
        }
    }

    #[test]
    fn roundtrip_and_screen() {
        let d = sample();
        let raw = d.encode();
        assert_eq!(Descriptor::decode(&raw).unwrap(), d);
        screen(&d).unwrap();
        let mut bad = d.clone();
        bad.reserved = "x".into();
        assert!(screen(&bad).is_err());
        let mut bad = d;
        bad.tree_cid = bad.tree_cid.to_uppercase();
        assert!(screen(&bad).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut raw = sample().encode().to_vec();
        raw.push(0x00);
        assert!(Descriptor::decode(&raw).is_err());
    }
}
