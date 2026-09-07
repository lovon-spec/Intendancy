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

/// The Runtimes column grammar (listing policy column 4, 2026-09-06): identifiers
/// `[a-z][a-z0-9_]{0,31}`, separated by single commas with no whitespace, unique,
/// in ascending byte order, at most 16; `generic` only on its own.
pub const MAX_RUNTIMES: usize = 16;
pub const MAX_RUNTIME_LEN: usize = 32;
pub fn check_runtimes(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("Runtimes must list at least one identifier".into());
    }
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() > MAX_RUNTIMES {
        return Err(format!(
            "Runtimes lists {} identifiers, at most {MAX_RUNTIMES} allowed",
            parts.len()
        ));
    }
    for (i, part) in parts.iter().enumerate() {
        let b = part.as_bytes();
        let well_formed = !b.is_empty()
            && b.len() <= MAX_RUNTIME_LEN
            && b[0].is_ascii_lowercase()
            && b.iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
        if !well_formed {
            return Err(format!(
                "Runtimes identifier {part:?} is not lowercase snake case (a letter, then letters, digits or underscores, at most {MAX_RUNTIME_LEN} characters)"
            ));
        }
        if i > 0 && parts[i - 1] >= *part {
            return Err(format!(
                "Runtimes identifiers must be unique and in ascending order ({:?} before {part:?})",
                parts[i - 1]
            ));
        }
    }
    if parts.len() > 1 && parts.contains(&"generic") {
        return Err("Runtimes: `generic` must be the only identifier when present".into());
    }
    Ok(())
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
    if let Err(e) = check_runtimes(&d.runtimes) {
        eyre::bail!("{e}");
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
    fn runtimes_grammar() {
        for ok in [
            "generic",
            "claude_code",
            "claude_code,cursor",
            "a,b",
            "codex,gemini_cli,openclaw",
            "x1_2",
        ] {
            assert!(check_runtimes(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "Generic",
            "claude code",
            "claude-code",
            "claude_code,",
            ",cursor",
            "cursor,claude_code",
            "cursor,cursor",
            "generic,cursor",
            "cursor,generic",
            "1abc",
            "_abc",
            "claude_code, cursor",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(check_runtimes(bad).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..17).map(|i| format!("r{i:02}")).collect();
        assert!(check_runtimes(&many.join(",")).is_err());
        assert!(check_runtimes(&many[..16].join(",")).is_ok());
        let mut d = sample();
        d.runtimes = "cursor,claude_code".into();
        assert!(screen(&d).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut raw = sample().encode().to_vec();
        raw.push(0x00);
        assert!(Descriptor::decode(&raw).is_err());
    }
}
