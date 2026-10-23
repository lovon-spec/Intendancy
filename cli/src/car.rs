//! CAR v1 + dag-pb + UnixFS consumer path implementing the PROVISIONAL installer
//! contract (spec §9) over the bounded **UnixFS-basic profile**: plain Directory
//! nodes (non-HAMT), single-block raw leaves, and kubo-style single-level
//! chunked File nodes; CIDv1 sha2-256/32; raw/dag-pb codecs only. HAMT shards
//! reject.
//!
//! SCOPE (deliberately narrow, PROVISIONAL): this is a **Kubo-interop
//! profile**, NOT general dag-pb/UnixFS spec conformance. The decoder accepts
//! either PBNode field order (Data/Links), any directory link ordering
//! (duplicate names still reject), absent link Names (read as empty), and
//! at-most-once mode/mtime metadata fields, whose VALUES are ignored
//! (executable bits are non-semantic by owner decision, listing-policy v2.1;
//! the mtime field's embedded UnixTime message is length-skipped, not
//! validated). Profile restrictions kept on purpose: the UnixFS Type field
//! must come FIRST (kubo emits fields in number order), and unknown/HAMT
//! fields reject. The ENCODER emits kubo's default byte layout
//! (`fixtures/kubo/`); directory links are emitted in CALLER order — canonical
//! name-sorting is the caller's responsibility (`build_dir` sorts).
//!
//! Hardening beyond the Gate 2 prototype (its documented residual list):
//! canonical-varint enforcement, duplicate-protobuf-field rejection, strict
//! PBLink field order, checked length arithmetic, tolerant-but-exact CAR header
//! decoding (either dag-cbor key order), block-count parse bound, 0700 staging
//! with EXCLUSIVE directory creation, atomic no-replace publish, and a
//! case/normalization coalescing guard that fails closed when bytewise-distinct
//! names would collide on insensitive filesystems.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use eyre::{bail, eyre, Context, Result};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

pub const MAX_BLOCK_BYTES: usize = 1024 * 1024;
pub const MAX_CAR_BYTES: u64 = 64 * 1024 * 1024;
/// Parse-time cap on the number of blocks in a CAR (review finding 9).
pub const MAX_CAR_BLOCKS: usize = 65_536;
pub const MAX_DEPTH: usize = 32;
pub const MAX_FILES: usize = 10_000;
/// Listing-policy aggregate tree cap, counted over MATERIALIZED bytes (a shared
/// block counts once per file it becomes), enforced during preflight.
pub const POLICY_MAX_TREE_BYTES: u64 = 2 * 1024 * 1024;

const CODEC_RAW: u8 = 0x55;
const CODEC_DAG_PB: u8 = 0x70;

/// Binary CIDv1 (sha2-256/32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cid {
    pub codec: u8,
    pub digest: [u8; 32],
}

impl Cid {
    pub fn for_block(codec: u8, data: &[u8]) -> Self {
        Self {
            codec,
            digest: Sha256::digest(data).into(),
        }
    }

    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = Vec::with_capacity(36);
        b.extend_from_slice(&[0x01, self.codec, 0x12, 0x20]);
        b.extend_from_slice(&self.digest);
        b
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        if raw.len() != 36 || raw[0] != 0x01 || raw[2] != 0x12 || raw[3] != 0x20 {
            bail!("unsupported CID form (require CIDv1 sha2-256/32)");
        }
        let codec = raw[1];
        if codec != CODEC_RAW && codec != CODEC_DAG_PB {
            bail!("CID codec {codec:#x} outside the raw/dag-pb allowlist");
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&raw[4..]);
        Ok(Self { codec, digest })
    }

    /// The 34-byte CIDv0 binary form (sha2-256 multihash, dag-pb implied), which
    /// legacy Kleros lists link to. Kept apart from `from_bytes`: the skills
    /// registry's policy requires CIDv1 everywhere, and `intend`'s tree walk
    /// must keep rejecting a v0 link.
    pub fn from_bytes_v0(raw: &[u8]) -> Result<Self> {
        if raw.len() != 34 || raw[0] != 0x12 || raw[1] != 0x20 {
            bail!("not a CIDv0 sha2-256/32 multihash");
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&raw[2..]);
        Ok(Self {
            codec: CODEC_DAG_PB,
            digest,
        })
    }

    /// Either binary form: CIDv1 (36 bytes) or CIDv0 (34 bytes).
    pub fn from_bytes_any(raw: &[u8]) -> Result<Self> {
        if raw.len() == 34 {
            Self::from_bytes_v0(raw)
        } else {
            Self::from_bytes(raw)
        }
    }

    /// Either text form: canonical CIDv1 base32 (`b…`) or CIDv0 base58btc
    /// (`Qm…`). Legacy lists carry CIDv0 paths; the registry itself does not.
    pub fn parse_any(s: &str) -> Result<Self> {
        if s.starts_with("Qm") {
            let raw = base58_decode(s).wrap_err("CIDv0 base58 decode")?;
            return Self::from_bytes_v0(&raw);
        }
        Self::parse_canonical(s)
    }

    /// Text form matching the CID's version: CIDv1 canonical base32, or the
    /// base58btc CIDv0 string when `v0` is requested for a dag-pb CID.
    pub fn to_string_v0(self) -> Result<String> {
        if self.codec != CODEC_DAG_PB {
            bail!("only dag-pb CIDs have a CIDv0 form");
        }
        let mut raw = vec![0x12, 0x20];
        raw.extend_from_slice(&self.digest);
        Ok(base58_encode(&raw))
    }

    /// Canonical text form (multibase base32 lower, no padding).
    pub fn to_string_canonical(self) -> String {
        format!(
            "b{}",
            data_encoding::BASE32_NOPAD
                .encode(&self.to_bytes())
                .to_lowercase()
        )
    }

    pub fn parse_canonical(s: &str) -> Result<Self> {
        let rest = s
            .strip_prefix('b')
            .ok_or_else(|| eyre!("CID must be multibase base32 ('b…')"))?;
        if rest != rest.to_lowercase() {
            bail!("CID must be lowercase");
        }
        let raw = data_encoding::BASE32_NOPAD
            .decode(rest.to_uppercase().as_bytes())
            .wrap_err("CID base32 decode")?;
        Self::from_bytes(&raw)
    }
}

// ---------- base58btc (CIDv0 text form only) ----------

const BASE58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Bitcoin-alphabet base58 decode, bounded to CID-sized inputs.
pub fn base58_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() > 64 {
        bail!("base58 input too long for a CID");
    }
    let mut out: Vec<u8> = Vec::new();
    for c in s.bytes() {
        let digit = BASE58_ALPHABET
            .iter()
            .position(|a| *a == c)
            .ok_or_else(|| eyre!("invalid base58 character {:?}", c as char))?
            as u32;
        let mut carry = digit;
        for byte in out.iter_mut().rev() {
            let v = u32::from(*byte) * 58 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let leading = s.bytes().take_while(|c| *c == b'1').count();
    let mut result = vec![0u8; leading];
    result.extend(out);
    Ok(result)
}

/// Bitcoin-alphabet base58 encode.
pub fn base58_encode(raw: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::new();
    for byte in raw {
        let mut carry = u32::from(*byte);
        for d in digits.iter_mut() {
            let v = u32::from(*d) * 256 + carry;
            *d = (v % 58) as u8;
            carry = v / 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut s = String::new();
    for _ in raw.iter().take_while(|b| **b == 0) {
        s.push('1');
    }
    for d in digits.iter().rev() {
        s.push(BASE58_ALPHABET[*d as usize] as char);
    }
    s
}

#[cfg(test)]
mod cidv0_tests {
    use super::{base58_decode, base58_encode, Cid};

    /// Vectors from kubo: `ipfs cid format -b base16 -v 1 <Qm…>`.
    #[test]
    fn cidv0_text_round_trips_through_the_kubo_vectors() {
        for (text, digest_hex) in [
            (
                "QmSgD2hjrA4jwTFP8GxR6zH9rc4GpG8CF7QF3xPzqycvxG",
                "4071540068a12010c59a4defd817025565edf0e61a9ca9a270081ad8d02d8831",
            ),
            (
                "QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv",
                "7f218c635c3a4d09afb01fab06272cb69a139318cc9c92f8062dd8f896284a6b",
            ),
        ] {
            let cid = Cid::parse_any(text).unwrap();
            assert_eq!(cid.codec, 0x70);
            assert_eq!(super::hex_lower(&cid.digest), digest_hex);
            assert_eq!(cid.to_string_v0().unwrap(), text);
            // The same CID in canonical v1 form parses back to the same value.
            assert_eq!(
                Cid::parse_canonical(&cid.to_string_canonical()).unwrap(),
                cid
            );
        }
    }

    #[test]
    fn base58_edge_cases() {
        assert_eq!(base58_encode(&[]), "");
        assert_eq!(base58_decode("").unwrap(), Vec::<u8>::new());
        assert_eq!(base58_encode(&[0, 0, 1]), "112");
        assert_eq!(base58_decode("112").unwrap(), vec![0, 0, 1]);
        assert!(base58_decode("0OIl").is_err());
        assert!(Cid::from_bytes_v0(&[0x12, 0x20]).is_err());
        assert!(
            Cid::from_bytes(&[0x12, 0x20]).is_err(),
            "the strict CIDv1 parser stays strict"
        );
    }
}

// ---------- varint (canonical) ----------

fn write_uvarint(out: &mut Vec<u8>, mut v: u64) {
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

/// Canonical unsigned varint: at most 10 bytes, no overflow past u64, and
/// MINIMAL — a continuation into a zero final group ("trailing zero padding")
/// rejects, so every value has exactly one accepted encoding.
fn read_uvarint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut out: u64 = 0;
    let mut consumed = 0usize;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*pos).ok_or_else(|| eyre!("varint truncated"))?;
        *pos += 1;
        consumed += 1;
        let group = u64::from(byte & 0x7f);
        if shift == 63 && group > 1 {
            bail!("varint overflows u64");
        }
        out |= group << shift;
        if byte & 0x80 == 0 {
            if consumed > 1 && byte == 0 {
                bail!("noncanonical varint (trailing zero group)");
            }
            return Ok(out);
        }
    }
    bail!("varint too long")
}

// ---------- dag-pb (PBNode/PBLink) + UnixFS Data ----------

#[derive(Debug, Clone)]
pub struct PbLink {
    pub cid: Cid,
    pub name: String,
    pub tsize: u64,
}

fn pb_field(out: &mut Vec<u8>, field: u64, wire: u64) {
    write_uvarint(out, (field << 3) | wire);
}

fn pb_bytes(out: &mut Vec<u8>, field: u64, data: &[u8]) {
    pb_field(out, field, 2);
    write_uvarint(out, data.len() as u64);
    out.extend_from_slice(data);
}

fn encode_link(link: &PbLink) -> Vec<u8> {
    let mut out = Vec::new();
    pb_bytes(&mut out, 1, &link.cid.to_bytes());
    pb_bytes(&mut out, 2, link.name.as_bytes());
    pb_field(&mut out, 3, 0);
    write_uvarint(&mut out, link.tsize);
    out
}

/// Directory PBNode in dag-pb canonical field order: Links (field 2) BEFORE
/// Data (field 1); UnixFS Data { Type = Directory(1) }. Links are emitted in
/// CALLER order — the canonical name-sorted form is the caller's precondition
/// (`build_dir` sorts; tests deliberately pass unsorted links to exercise
/// decoder acceptance of any ordering).
pub fn encode_directory(links: &[PbLink]) -> Vec<u8> {
    let mut out = Vec::new();
    for link in links {
        let l = encode_link(link);
        pb_bytes(&mut out, 2, &l);
    }
    let unixfs = vec![0x08, 0x01];
    pb_bytes(&mut out, 1, &unixfs);
    out
}

/// Chunked-file PBNode exactly as kubo's default layout emits it: empty-named
/// links to the raw chunks, then UnixFS Data { Type = File(2), filesize,
/// blocksizes[] } — fields in number order, no inline data.
pub fn encode_chunked_file(chunks: &[(Cid, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (cid, len) in chunks {
        let l = encode_link(&PbLink {
            cid: *cid,
            name: String::new(),
            tsize: *len,
        });
        pb_bytes(&mut out, 2, &l);
    }
    let mut unixfs = Vec::new();
    pb_field(&mut unixfs, 1, 0); // Type
    write_uvarint(&mut unixfs, UNIXFS_FILE);
    pb_field(&mut unixfs, 3, 0); // filesize
    write_uvarint(&mut unixfs, chunks.iter().map(|(_, l)| l).sum());
    for (_, len) in chunks {
        pb_field(&mut unixfs, 4, 0); // blocksizes
        write_uvarint(&mut unixfs, *len);
    }
    pb_bytes(&mut out, 1, &unixfs);
    out
}

pub const UNIXFS_FILE: u64 = 2;
pub const UNIXFS_DIRECTORY: u64 = 1;

/// The UnixFS Data message, decoded strictly: Type (1, required FIRST — a
/// profile restriction matching kubo's field-number-order emission), optional
/// inline Data bytes (2), optional filesize (3), repeated blocksizes (4).
/// Metadata mode (7) and mtime (8) are accepted at most once each and IGNORED
/// (mtime's embedded UnixTime is length-skipped, not validated). HAMT fields
/// (hashType 5 / fanout 6) and anything else are OUTSIDE the profile and
/// reject.
#[derive(Debug)]
pub struct UnixFs {
    pub node_type: u64,
    pub data: Option<Vec<u8>>,
    pub filesize: Option<u64>,
    pub blocksizes: Vec<u64>,
}

#[derive(Debug)]
pub struct PbNode {
    pub links: Vec<PbLink>,
    pub unixfs: UnixFs,
}

fn decode_unixfs(body: &[u8]) -> Result<UnixFs> {
    let mut p = 0usize;
    // Require Type first (kubo emits fields in number order; Type is field 1).
    let key = read_uvarint(body, &mut p)?;
    if key >> 3 != 1 || key & 7 != 0 {
        bail!("UnixFS Data does not start with Type");
    }
    let node_type = read_uvarint(body, &mut p)?;
    let mut data = None;
    let mut filesize = None;
    let mut blocksizes = Vec::new();
    let mut saw_mode = false;
    let mut saw_mtime = false;
    while p < body.len() {
        let key = read_uvarint(body, &mut p)?;
        match (key >> 3, key & 7) {
            (2, 2) => {
                if data.is_some() {
                    bail!("duplicate UnixFS Data bytes field");
                }
                let len = read_uvarint(body, &mut p)? as usize;
                let end = p.checked_add(len).ok_or_else(|| eyre!("overflow"))?;
                data = Some(
                    body.get(p..end)
                        .ok_or_else(|| eyre!("UnixFS Data truncated"))?
                        .to_vec(),
                );
                p = end;
            }
            (3, 0) => {
                if filesize.is_some() {
                    bail!("duplicate UnixFS filesize field");
                }
                filesize = Some(read_uvarint(body, &mut p)?);
            }
            (4, 0) => blocksizes.push(read_uvarint(body, &mut p)?),
            // mode (7, varint) and mtime (8, embedded message) are legal UnixFS
            // metadata: accepted at most ONCE each and IGNORED (exec bits are
            // non-semantic by owner decision; times are transport-only; the
            // mtime UnixTime message is length-skipped, not validated).
            (7, 0) => {
                if saw_mode {
                    bail!("duplicate UnixFS mode field");
                }
                saw_mode = true;
                let _ = read_uvarint(body, &mut p)?;
            }
            (8, 2) => {
                if saw_mtime {
                    bail!("duplicate UnixFS mtime field");
                }
                saw_mtime = true;
                let len = read_uvarint(body, &mut p)? as usize;
                let end = p.checked_add(len).ok_or_else(|| eyre!("overflow"))?;
                body.get(p..end)
                    .ok_or_else(|| eyre!("UnixFS mtime truncated"))?;
                p = end;
            }
            (f, _) => bail!(
                "UnixFS field {f} outside the UnixFS-basic profile (HAMT shard \
                 fields reject)"
            ),
        }
    }
    Ok(UnixFs {
        node_type,
        data,
        filesize,
        blocksizes,
    })
}

pub fn decode_pbnode(raw: &[u8]) -> Result<PbNode> {
    decode_pbnode_with(raw, false)
}

/// `decode_pbnode` that also accepts CIDv0 link hashes (34-byte multihash
/// form). Legacy Kleros lists link their item files this way; the skills
/// registry's strict walk keeps using `decode_pbnode`.
pub fn decode_pbnode_lenient(raw: &[u8]) -> Result<PbNode> {
    decode_pbnode_with(raw, true)
}

fn decode_pbnode_with(raw: &[u8], allow_v0_links: bool) -> Result<PbNode> {
    let mut pos = 0usize;
    let mut links = Vec::new();
    let mut unixfs = None;
    while pos < raw.len() {
        let key = read_uvarint(raw, &mut pos)?;
        let field = key >> 3;
        let wire = key & 0x7;
        if wire != 2 {
            bail!("unexpected wire type {wire} in PBNode field {field}");
        }
        let len = read_uvarint(raw, &mut pos)? as usize;
        let end = pos.checked_add(len).ok_or_else(|| eyre!("overflow"))?;
        let body = raw.get(pos..end).ok_or_else(|| eyre!("PBNode truncated"))?;
        pos = end;
        match field {
            2 => {
                // Per the dag-pb spec, DECODERS accept either PBNode field
                // order (our encoder still emits Links-then-Data canonically).
                links.push(decode_link(body, allow_v0_links)?);
            }
            1 => {
                if unixfs.is_some() {
                    bail!("duplicate PBNode Data field");
                }
                unixfs = Some(decode_unixfs(body)?);
            }
            other => bail!("unexpected PBNode field {other}"),
        }
    }
    Ok(PbNode {
        links,
        unixfs: unixfs.ok_or_else(|| eyre!("PBNode lacks UnixFS Data"))?,
    })
}

fn decode_link(raw: &[u8], allow_v0: bool) -> Result<PbLink> {
    let mut pos = 0usize;
    let mut cid = None;
    let mut name = None;
    let mut tsize = None;
    let mut last_field = 0u64;
    while pos < raw.len() {
        let key = read_uvarint(raw, &mut pos)?;
        let field = key >> 3;
        // dag-pb strict encoding: PBLink fields appear at most once, in
        // ascending field order (Hash 1, Name 2, Tsize 3) — this also makes the
        // duplicate checks below structural rather than incidental.
        if field <= last_field {
            bail!("PBLink field {field} out of order or duplicated");
        }
        last_field = field;
        match (field, key & 7) {
            (1, 2) => {
                let len = read_uvarint(raw, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or_else(|| eyre!("overflow"))?;
                let cid_bytes = raw.get(pos..end).ok_or_else(|| eyre!("link truncated"))?;
                cid = Some(if allow_v0 {
                    Cid::from_bytes_any(cid_bytes)?
                } else {
                    Cid::from_bytes(cid_bytes)?
                });
                pos = end;
            }
            (2, 2) => {
                let len = read_uvarint(raw, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or_else(|| eyre!("overflow"))?;
                name = Some(
                    std::str::from_utf8(raw.get(pos..end).ok_or_else(|| eyre!("link truncated"))?)
                        .wrap_err("link name utf8")?
                        .to_string(),
                );
                pos = end;
            }
            (3, 0) => {
                tsize = Some(read_uvarint(raw, &mut pos)?);
            }
            (f, w) => bail!("unexpected PBLink field {f} wire {w}"),
        }
    }
    Ok(PbLink {
        cid: cid.ok_or_else(|| eyre!("PBLink lacks Hash"))?,
        // Per current dag-pb practice an ABSENT Name field reads as the empty
        // name (review finding 7) — name presence is not semantic; DIRECTORY
        // entries still require non-empty sanitized names at the walk layer.
        name: name.unwrap_or_default(),
        tsize: tsize.unwrap_or(0),
    })
}

// ---------- CAR v1 ----------

/// dag-cbor header {"roots":[CID], "version": 1} — emitted in canonical key
/// order ("roots" before "version").
fn encode_car_header(root: Cid) -> Vec<u8> {
    let mut h = Vec::new();
    h.push(0xa2); // map(2)
    h.extend_from_slice(&[0x65]); // text(5)
    h.extend_from_slice(b"roots");
    h.push(0x81); // array(1)
    h.extend_from_slice(&[0xd8, 0x2a]); // tag(42)
    let cid_bytes = root.to_bytes();
    h.push(0x58); // bytes(len8)
    h.push((cid_bytes.len() + 1) as u8);
    h.push(0x00); // multibase identity prefix
    h.extend_from_slice(&cid_bytes);
    h.extend_from_slice(&[0x67]); // text(7)
    h.extend_from_slice(b"version");
    h.push(0x01); // 1
    h
}

/// Decode a CAR v1 header: exactly the map {"roots":[one CID], "version":1},
/// tolerating either key order (kubo/go-car emit canonical order; the bench
/// historically emitted the same, but the decoder must not depend on it), and
/// nothing else. Any other shape fails closed.
fn decode_car_header(raw: &[u8]) -> Result<Cid> {
    let mut pos = 0usize;
    let next = |raw: &[u8], pos: &mut usize| -> Result<u8> {
        let b = *raw.get(*pos).ok_or_else(|| eyre!("CAR header truncated"))?;
        *pos += 1;
        Ok(b)
    };
    if next(raw, &mut pos)? != 0xa2 {
        bail!("CAR header is not a two-entry map");
    }
    let mut root: Option<Cid> = None;
    let mut version_ok = false;
    for _ in 0..2 {
        let key_head = next(raw, &mut pos)?;
        let key_len = match key_head {
            0x60..=0x77 => (key_head - 0x60) as usize,
            _ => bail!("unsupported CAR header key encoding"),
        };
        let end = pos.checked_add(key_len).ok_or_else(|| eyre!("overflow"))?;
        let key = raw
            .get(pos..end)
            .ok_or_else(|| eyre!("CAR header truncated"))?;
        pos = end;
        match key {
            b"roots" => {
                if root.is_some() {
                    bail!("duplicate roots key");
                }
                if next(raw, &mut pos)? != 0x81 {
                    bail!("CAR header must carry exactly one root");
                }
                if next(raw, &mut pos)? != 0xd8 || next(raw, &mut pos)? != 0x2a {
                    bail!("root is not a CID (tag 42)");
                }
                if next(raw, &mut pos)? != 0x58 {
                    bail!("unsupported CID byte-string encoding");
                }
                let blen = next(raw, &mut pos)? as usize;
                let end = pos.checked_add(blen).ok_or_else(|| eyre!("overflow"))?;
                let body = raw
                    .get(pos..end)
                    .ok_or_else(|| eyre!("CAR header truncated"))?;
                pos = end;
                if body.first() != Some(&0x00) {
                    bail!("CID in header lacks identity prefix");
                }
                root = Some(Cid::from_bytes(&body[1..])?);
            }
            b"version" => {
                if version_ok {
                    bail!("duplicate version key");
                }
                if next(raw, &mut pos)? != 0x01 {
                    bail!("unsupported CAR version (require 1)");
                }
                version_ok = true;
            }
            other => bail!(
                "unexpected CAR header key {:?}",
                String::from_utf8_lossy(other)
            ),
        }
    }
    if pos != raw.len() {
        bail!("trailing bytes after CAR header map");
    }
    if !version_ok {
        bail!("CAR header lacks version");
    }
    root.ok_or_else(|| eyre!("CAR header lacks roots"))
}

pub fn write_car(root: Cid, blocks: &BTreeMap<Cid, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let header = encode_car_header(root);
    write_uvarint(&mut out, header.len() as u64);
    out.extend_from_slice(&header);
    for (cid, data) in blocks {
        let cid_bytes = cid.to_bytes();
        write_uvarint(&mut out, (cid_bytes.len() + data.len()) as u64);
        out.extend_from_slice(&cid_bytes);
        out.extend_from_slice(data);
    }
    out
}

/// Parse a CAR v1 byte stream: bounded, every block hash-checked at read time,
/// duplicates rejected (a CAR carries each block once; LINKS may share freely).
pub fn read_car(raw: &[u8]) -> Result<(Cid, BTreeMap<Cid, Vec<u8>>)> {
    if raw.len() as u64 > MAX_CAR_BYTES {
        bail!("CAR exceeds {MAX_CAR_BYTES} byte bound");
    }
    let mut pos = 0usize;
    let hlen = read_uvarint(raw, &mut pos)? as usize;
    let hend = pos.checked_add(hlen).ok_or_else(|| eyre!("overflow"))?;
    let header = raw
        .get(pos..hend)
        .ok_or_else(|| eyre!("CAR header truncated"))?;
    pos = hend;
    let root = decode_car_header(header)?;
    let mut blocks = BTreeMap::new();
    while pos < raw.len() {
        if blocks.len() >= MAX_CAR_BLOCKS {
            bail!("CAR exceeds the {MAX_CAR_BLOCKS}-block bound");
        }
        let blen = read_uvarint(raw, &mut pos)? as usize;
        if !(36..=36 + MAX_BLOCK_BYTES).contains(&blen) {
            bail!("block length {blen} outside bounds");
        }
        let end = pos.checked_add(blen).ok_or_else(|| eyre!("overflow"))?;
        let body = raw.get(pos..end).ok_or_else(|| eyre!("block truncated"))?;
        pos = end;
        let cid = Cid::from_bytes(&body[..36])?;
        let data = &body[36..];
        if Cid::for_block(cid.codec, data) != cid {
            bail!(
                "block {} does not hash its contents",
                cid.to_string_canonical()
            );
        }
        if blocks.insert(cid, data.to_vec()).is_some() {
            bail!("duplicate block {}", cid.to_string_canonical());
        }
    }
    Ok((root, blocks))
}

// ---------- build (provider/test side) ----------

/// kubo's default chunker size — files larger than this become a File node over
/// 256 KiB raw chunks, exactly as `ipfs add --cid-version 1` produces.
pub const KUBO_CHUNK_BYTES: usize = 262_144;

/// Build a UnixFS-basic DAG from a directory, byte-identical to kubo's default
/// `ipfs add --cid-version 1` output (proven by `fixtures/kubo/`): files up to
/// 256 KiB are single raw blocks; larger files chunk into 256 KiB raw blocks
/// under a File node; directories nest, entries sorted by name.
pub fn build_dir(path: &Path, blocks: &mut BTreeMap<Cid, Vec<u8>>) -> Result<(Cid, u64)> {
    let mut entries: Vec<_> = std::fs::read_dir(path)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    let mut links = Vec::new();
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| eyre!("non-UTF-8 file name"))?;
        let ftype = entry.file_type()?;
        if ftype.is_symlink() {
            bail!("symlinks are not supported in skill trees");
        }
        if ftype.is_dir() {
            let (cid, tsize) = build_dir(&entry.path(), blocks)?;
            links.push(PbLink { cid, name, tsize });
        } else if !ftype.is_file() {
            // A FIFO/socket/device is never part of a skill tree — and reading
            // one could hang; fail typed instead (listing-policy v2.1: trees
            // are directories and regular files only).
            bail!("unsupported node kind at {name:?} (directories and regular files only)");
        } else {
            let data = std::fs::read(entry.path())?;
            if data.len() <= KUBO_CHUNK_BYTES {
                let cid = Cid::for_block(CODEC_RAW, &data);
                let tsize = data.len() as u64;
                blocks.insert(cid, data);
                links.push(PbLink { cid, name, tsize });
            } else {
                if data.len() as u64 > POLICY_MAX_TREE_BYTES {
                    bail!("file {name} exceeds the {POLICY_MAX_TREE_BYTES}-byte policy cap");
                }
                let mut chunks = Vec::new();
                for chunk in data.chunks(KUBO_CHUNK_BYTES) {
                    let cid = Cid::for_block(CODEC_RAW, chunk);
                    blocks.insert(cid, chunk.to_vec());
                    chunks.push((cid, chunk.len() as u64));
                }
                let node = encode_chunked_file(&chunks);
                let tsize = node.len() as u64 + data.len() as u64;
                let cid = Cid::for_block(CODEC_DAG_PB, &node);
                blocks.insert(cid, node);
                links.push(PbLink { cid, name, tsize });
            }
        }
    }
    let node = encode_directory(&links);
    let tsize = node.len() as u64 + links.iter().map(|l| l.tsize).sum::<u64>();
    let cid = Cid::for_block(CODEC_DAG_PB, &node);
    blocks.insert(cid, node);
    Ok((cid, tsize))
}

// ---------- verify + sanitized install (consumer side, spec §9 contract) ----------

fn sanitize_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." {
        bail!("unsafe path component {name:?}");
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("unsafe path component {name:?}");
    }
    let as_path = Path::new(name);
    if as_path.components().count() != 1
        || !matches!(as_path.components().next(), Some(Component::Normal(_)))
    {
        bail!("unsafe path component {name:?}");
    }
    Ok(())
}

/// Case/normalization fold for the coalescing guard: two entry paths whose folds
/// collide would land on the same file on case-insensitive or
/// normalization-insensitive filesystems (APFS default) — the §9 contract
/// requires failing closed on that, so the fold is deliberately broad
/// (NFC + lowercase).
fn fold_path(p: &Path) -> String {
    p.to_string_lossy().nfc().collect::<String>().to_lowercase()
}

/// The fully validated result of a preflight walk — everything `install` will
/// write, decided BEFORE any filesystem mutation.
#[derive(Debug)]
pub struct InstallPlan {
    pub dirs: Vec<PathBuf>,
    pub files: Vec<PlannedFile>,
    /// Materialized bytes (shared blocks counted once per planned file).
    pub total_bytes: u64,
    pub blocks: u64,
}

#[derive(Debug)]
pub struct PlannedFile {
    pub rel: PathBuf,
    pub source: FileSource,
    pub bytes: u64,
    pub sha256: String,
}

/// Where a planned file's bytes come from — a single raw block, the ordered
/// chunks of a chunked File node, or a File node's inline Data bytes.
#[derive(Debug)]
pub enum FileSource {
    Raw(Cid),
    Chunks(Vec<ChunkRef>),
    Inline(Vec<u8>),
}

/// One chunk of a chunked File: a raw block (raw-leaves producers, kubo's
/// default) or a leaf File node whose content is its inline Data (raw-leaves-off
/// producers, e.g. kubo --preserve-mode).
#[derive(Debug)]
pub enum ChunkRef {
    Raw(Cid),
    FileInline(Cid),
}

/// The verified content bytes a chunk contributes.
fn chunk_bytes(chunk: &ChunkRef, blocks: &BTreeMap<Cid, Vec<u8>>) -> Result<Vec<u8>> {
    match chunk {
        ChunkRef::Raw(cid) => Ok(blocks
            .get(cid)
            .ok_or_else(|| eyre!("missing block"))?
            .clone()),
        ChunkRef::FileInline(cid) => {
            let node = decode_pbnode(blocks.get(cid).ok_or_else(|| eyre!("missing block"))?)?;
            Ok(node.unixfs.data.unwrap_or_default())
        }
    }
}

impl InstallPlan {
    pub fn locked_files(&self) -> Vec<crate::lockfile::LockedFile> {
        self.files
            .iter()
            .map(|f| crate::lockfile::LockedFile {
                path: f.rel.to_string_lossy().into_owned(),
                bytes: f.bytes,
                sha256: f.sha256.clone(),
            })
            .collect()
    }
}

/// Fetch a block from the untrusted map, REHASHING it on first visit: the map key
/// is not evidence — only the hash of the bytes is.
fn fetch_verified<'a>(
    cid: Cid,
    blocks: &'a BTreeMap<Cid, Vec<u8>>,
    used: &mut std::collections::BTreeSet<Cid>,
) -> Result<&'a Vec<u8>> {
    let data = blocks
        .get(&cid)
        .ok_or_else(|| eyre!("missing block {}", cid.to_string_canonical()))?;
    if used.insert(cid) && Cid::for_block(cid.codec, data) != cid {
        bail!(
            "block {} does not hash its contents",
            cid.to_string_canonical()
        );
    }
    Ok(data)
}

/// Validate the complete DAG under `expected_root` WITHOUT touching the
/// filesystem: the root must equal the externally supplied Tree CID, every block
/// must be reachable, names sanitized (with the coalescing guard), depth/count
/// and the materialized-byte policy cap enforced as the plan grows.
pub fn preflight(expected_root: Cid, blocks: &BTreeMap<Cid, Vec<u8>>) -> Result<InstallPlan> {
    if expected_root.codec != CODEC_DAG_PB {
        bail!("tree root must be dag-pb");
    }
    if !blocks.contains_key(&expected_root) {
        bail!(
            "CAR does not contain the expected root {} (root substitution or wrong CAR)",
            expected_root.to_string_canonical()
        );
    }
    let mut used: std::collections::BTreeSet<Cid> = std::collections::BTreeSet::new();
    let mut folded: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut plan = InstallPlan {
        dirs: Vec::new(),
        files: Vec::new(),
        total_bytes: 0,
        blocks: blocks.len() as u64,
    };
    walk_plan(
        expected_root,
        blocks,
        PathBuf::new(),
        0,
        &mut used,
        &mut folded,
        &mut plan,
    )?;
    if used.len() != blocks.len() {
        bail!(
            "CAR contains {} block(s) unreachable from the root (complete-DAG rule)",
            blocks.len() - used.len()
        );
    }
    plan.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    plan.dirs.sort();
    plan.dirs.dedup();
    Ok(plan)
}

fn walk_plan(
    node: Cid,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    rel: PathBuf,
    depth: usize,
    used: &mut std::collections::BTreeSet<Cid>,
    folded: &mut std::collections::BTreeSet<String>,
    plan: &mut InstallPlan,
) -> Result<()> {
    let raw = fetch_verified(node, blocks, used)?;
    let pb = decode_pbnode(raw)?;
    if pb.unixfs.node_type != UNIXFS_DIRECTORY {
        bail!(
            "expected a UnixFS Directory node, got type {} (the tree root and every \
             directory must be a plain Directory; HAMT shards are outside the \
             UnixFS-basic profile)",
            pb.unixfs.node_type
        );
    }
    if pb.unixfs.data.is_some() || pb.unixfs.filesize.is_some() || !pb.unixfs.blocksizes.is_empty()
    {
        bail!("Directory node carries File fields");
    }
    walk_dir_entries(pb.links, blocks, rel, depth, used, folded, plan)
}

fn walk_dir_entries(
    links: Vec<PbLink>,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    rel: PathBuf,
    depth: usize,
    used: &mut std::collections::BTreeSet<Cid>,
    folded: &mut std::collections::BTreeSet<String>,
    plan: &mut InstallPlan,
) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("tree exceeds depth bound {MAX_DEPTH}");
    }
    let mut seen = std::collections::BTreeSet::new();
    for link in links {
        sanitize_name(&link.name)?;
        // Per the UnixFS spec, decoders accept ANY directory link ordering;
        // duplicate names still reject.
        if !seen.insert(link.name.clone()) {
            bail!("duplicate entry name {:?}", link.name);
        }
        let child_rel = rel.join(&link.name);
        // Coalescing guard: bytewise-distinct paths whose fold collides would
        // merge on case/normalization-insensitive filesystems — fail closed.
        if !folded.insert(fold_path(&child_rel)) {
            bail!(
                "entry {:?} collides with another entry under case/Unicode folding \
                 (would coalesce on case-insensitive filesystems)",
                child_rel
            );
        }
        match link.cid.codec {
            CODEC_RAW => {
                let data = fetch_verified(link.cid, blocks, used)?;
                push_file(
                    plan,
                    child_rel,
                    FileSource::Raw(link.cid),
                    data.len() as u64,
                    Sha256::digest(data).into(),
                )?;
            }
            CODEC_DAG_PB => {
                let child_raw = fetch_verified(link.cid, blocks, used)?;
                let child = decode_pbnode(child_raw)?;
                match child.unixfs.node_type {
                    UNIXFS_DIRECTORY => {
                        if plan.dirs.len() >= MAX_FILES {
                            bail!("tree exceeds directory bound {MAX_FILES}");
                        }
                        if child.unixfs.data.is_some()
                            || child.unixfs.filesize.is_some()
                            || !child.unixfs.blocksizes.is_empty()
                        {
                            bail!("Directory node carries File fields");
                        }
                        plan.dirs.push(child_rel.clone());
                        walk_dir_entries(
                            child.links,
                            blocks,
                            child_rel,
                            depth + 1,
                            used,
                            folded,
                            plan,
                        )?;
                    }
                    UNIXFS_FILE => {
                        plan_chunked_file(&child, blocks, used, plan, child_rel)?;
                    }
                    other => bail!(
                        "UnixFS node type {other} outside the UnixFS-basic profile \
                         (directories and files only)"
                    ),
                }
            }
            other => bail!("link codec {other:#x} outside allowlist"),
        }
    }
    Ok(())
}

fn push_file(
    plan: &mut InstallPlan,
    rel: PathBuf,
    source: FileSource,
    bytes: u64,
    digest: [u8; 32],
) -> Result<()> {
    if plan.files.len() >= MAX_FILES {
        bail!("tree exceeds file bound {MAX_FILES}");
    }
    plan.total_bytes += bytes;
    if plan.total_bytes > POLICY_MAX_TREE_BYTES {
        bail!("materialized tree exceeds the {POLICY_MAX_TREE_BYTES}-byte policy cap");
    }
    plan.files.push(PlannedFile {
        rel,
        source,
        bytes,
        sha256: hex_lower(&digest),
    });
    Ok(())
}

/// Validate and plan a kubo-style File node. Every chunk link must carry an
/// EMPTY name and be either a raw block (raw-leaves producers, kubo default)
/// or a LEAF File node whose content is inline Data (raw-leaves-off producers,
/// e.g. kubo --preserve-mode); blocksizes must match the chunk lengths one for
/// one and sum to the declared filesize. DEEPER nesting (chunks with their own
/// links) is outside the UnixFS-basic profile — at the 2 MiB policy cap a
/// single-level layout always suffices (kubo only nests beyond ~45 MB).
fn plan_chunked_file(
    node: &PbNode,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    used: &mut std::collections::BTreeSet<Cid>,
    plan: &mut InstallPlan,
    rel: PathBuf,
) -> Result<()> {
    let fs = &node.unixfs;
    if node.links.is_empty() {
        // Inline-data File node (legal UnixFS; kubo emits them for small files
        // when metadata forces a File wrapper, e.g. --preserve-mode, and for
        // empty files). Content = the Data bytes (possibly none).
        if !fs.blocksizes.is_empty() {
            bail!("File node with inline data must not carry blocksizes");
        }
        let data = fs.data.clone().unwrap_or_default();
        if let Some(size) = fs.filesize {
            if size != data.len() as u64 {
                bail!(
                    "File node filesize {size} != inline data length {}",
                    data.len()
                );
            }
        }
        let bytes = data.len() as u64;
        let digest: [u8; 32] = Sha256::digest(&data).into();
        return push_file(plan, rel, FileSource::Inline(data), bytes, digest);
    }
    if fs.data.is_some() {
        bail!("chunked File node must not carry inline data");
    }
    let filesize = fs
        .filesize
        .ok_or_else(|| eyre!("chunked File node lacks filesize"))?;
    if fs.blocksizes.len() != node.links.len() {
        bail!(
            "File node has {} blocksizes for {} chunks",
            fs.blocksizes.len(),
            node.links.len()
        );
    }
    let mut chunks = Vec::with_capacity(node.links.len());
    let mut total = 0u64;
    let mut hasher = Sha256::new();
    for (link, &declared) in node.links.iter().zip(&fs.blocksizes) {
        if !link.name.is_empty() {
            bail!("File chunk link carries a name {:?}", link.name);
        }
        // A chunk is either a raw block (raw-leaves producers) or a LEAF File
        // node carrying inline Data (raw-leaves-off producers). Deeper nesting
        // (chunks with their own links) stays outside the profile — impossible
        // to need under the 2 MiB policy cap.
        let (content, chunk_ref): (Vec<u8>, ChunkRef) = match link.cid.codec {
            CODEC_RAW => {
                let data = fetch_verified(link.cid, blocks, used)?;
                (data.clone(), ChunkRef::Raw(link.cid))
            }
            CODEC_DAG_PB => {
                let raw = fetch_verified(link.cid, blocks, used)?;
                let child = decode_pbnode(raw)?;
                if child.unixfs.node_type != UNIXFS_FILE {
                    bail!("File chunk is a non-File dag-pb node");
                }
                if !child.links.is_empty() || !child.unixfs.blocksizes.is_empty() {
                    bail!(
                        "multi-level File chunk trees are outside the UnixFS-basic \
                         profile"
                    );
                }
                let data = child.unixfs.data.unwrap_or_default();
                if let Some(size) = child.unixfs.filesize {
                    if size != data.len() as u64 {
                        bail!(
                            "chunk File node filesize {size} != inline data length {}",
                            data.len()
                        );
                    }
                }
                (data, ChunkRef::FileInline(link.cid))
            }
            other => bail!("File chunk codec {other:#x} outside allowlist"),
        };
        if content.len() as u64 != declared {
            bail!(
                "File chunk length {} != declared blocksize {declared}",
                content.len()
            );
        }
        total = total
            .checked_add(declared)
            .ok_or_else(|| eyre!("File size overflow"))?;
        if total > POLICY_MAX_TREE_BYTES {
            bail!("materialized tree exceeds the {POLICY_MAX_TREE_BYTES}-byte policy cap");
        }
        hasher.update(&content);
        chunks.push(chunk_ref);
    }
    if total != filesize {
        bail!("File chunks sum to {total}, declared filesize is {filesize}");
    }
    let digest: [u8; 32] = hasher.finalize().into();
    push_file(plan, rel, FileSource::Chunks(chunks), filesize, digest)
}

// ---------- bound install target + fd-relative staged publish ----------

/// A BOUND install destination (spec §9): the canonical parent directory is
/// resolved ONCE and then held open as a directory fd — staging, file writes,
/// and the publish rename are all performed RELATIVE TO THAT FD, so
/// retargeting the parent PATH after binding (e.g. swapping a path component
/// for a symlink) cannot redirect where content lands. `path()` is the
/// canonical destination callers journal BEFORE publishing; `publish` lands on
/// exactly this binding, never on a re-resolution of the original argument.
///
/// Residual (named §9 production blocker, shared with the audit walk): the
/// bind itself is canonicalize-then-open — narrowed by O_NOFOLLOW plus a
/// dev/ino cross-check, but not a fully race-free resolution — and a local
/// attacker who OWNS the bound parent directory can still rename entries
/// within it (such publishes fail CLOSED via the no-replace rename; they are
/// not silently redirected).
pub struct InstallTarget {
    parent: std::fs::File,
    parent_path: PathBuf,
    name: String,
    path: PathBuf,
}

impl InstallTarget {
    /// Bind `dest`: validate the final component, canonicalize the parent,
    /// open it as a directory fd, and cross-check the fd against the path.
    /// Paths are required to be valid UTF-8 so the lockfile records them
    /// EXACTLY (never lossily).
    pub fn bind(dest: &Path) -> Result<Self> {
        let name = dest
            .file_name()
            .ok_or_else(|| {
                eyre!(
                    "destination {} has no usable final component",
                    dest.display()
                )
            })?
            .to_str()
            .ok_or_else(|| {
                eyre!("destination name must be valid UTF-8 (the lockfile records exact paths)")
            })?
            .to_string();
        sanitize_name(&name)?;
        let parent = match dest.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let parent_path = parent
            .canonicalize()
            .wrap_err_with(|| format!("destination parent {} must exist", parent.display()))?;
        if parent_path.to_str().is_none() {
            bail!("destination parent path must be valid UTF-8 (the lockfile records exact paths)");
        }
        let parent_fd = open_dirfd(&parent_path)?;
        let path = parent_path.join(&name);
        let target = Self {
            parent: parent_fd,
            parent_path,
            name,
            path,
        };
        // Narrow the canonicalize→open race: the fd must denote the very
        // directory object the canonical path named.
        target.verify_identity()?;
        Ok(target)
    }

    /// Re-compare the CURRENT parent pathname against the held fd (dev/ino,
    /// no-follow). Called at bind, at publish entry, immediately before the
    /// publish rename, and by callers before FINALIZING the lockfile record
    /// (round-4 P1): the installer must never knowingly proceed — or finalize
    /// a record — while the recorded pathname no longer names the bound
    /// directory. This is a fail-closed spot check, not a race-free guarantee
    /// (named §9 residual).
    pub fn verify_identity(&self) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        let opened = self.parent.metadata().wrap_err("fstat bound parent")?;
        let named = std::fs::symlink_metadata(&self.parent_path)
            .wrap_err_with(|| format!("stat parent path {}", self.parent_path.display()))?;
        if named.file_type().is_symlink()
            || opened.dev() != named.dev()
            || opened.ino() != named.ino()
        {
            bail!(
                "destination parent {} no longer names the bound directory (path \
                 retargeted mid-transaction) — failing closed",
                self.parent_path.display()
            );
        }
        Ok(())
    }

    /// The canonical destination this target publishes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The canonical destination as the EXACT string the lockfile records
    /// (`bind` enforces UTF-8).
    pub fn path_str(&self) -> &str {
        self.path.to_str().expect("bind() enforces UTF-8 paths")
    }

    /// Require that nothing exists at the destination NAME in the bound parent
    /// (checked without following symlinks). The atomic no-replace rename at
    /// publish stays authoritative for races — this check makes a repeat
    /// install fail BEFORE any journaling touches the lockfile.
    pub fn check_vacant(&self) -> Result<()> {
        match fstatat_nofollow(&self.parent, &self.name) {
            Ok(_) => bail!(
                "destination {} already exists (file, directory, or symlink) — refusing to replace",
                self.path.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).wrap_err("stat destination"),
        }
    }
}

/// Finalization durability (round-4 P0): a crash between the publish rename
/// and the parent-directory fsync — or a failed parent fsync after a
/// successful rename — leaves VISIBLE content whose directory entry may not
/// survive power loss, while the full-manifest journal makes it recoverable.
/// Before a pending journal is cleared, the finalizer calls this: re-bind the
/// parent of the EXISTING destination, require a real directory at the bound
/// name (no-follow), and fsync the parent so the dirent is durable. Only then
/// may `pending` clear.
pub fn ensure_durable_destination(dest: &Path) -> Result<()> {
    let target = InstallTarget::bind(dest)
        .wrap_err("re-binding the published destination for durability")?;
    let st = fstatat_nofollow(&target.parent, &target.name)
        .wrap_err_with(|| format!("published destination {} is missing", target.path.display()))?;
    if st.st_mode & libc::S_IFMT != libc::S_IFDIR {
        bail!(
            "published destination {} is not a directory (no-follow) — refusing to finalize",
            target.path.display()
        );
    }
    target
        .parent
        .sync_all()
        .wrap_err("fsync of the install parent (publish-rename durability)")?;
    Ok(())
}

/// Publish a verified plan into the BOUND target: stage into a fresh private
/// (0700) directory created inside the held parent fd, write every planned
/// file (create-new, no-follow), make the content DURABLE (fsync every file,
/// every directory, then the stage root), then ONE atomic no-replace rename of
/// the stage onto the destination name — also fd-relative — followed by an
/// fsync of the parent directory, so the published name is durable before the
/// caller's finalizing lockfile save. On error the staging directory is
/// removed (best effort) and the destination is untouched; if the error
/// follows the rename itself (parent fsync), content IS published and the
/// caller's pending journal covers it.
pub fn publish(
    plan: &InstallPlan,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    target: &InstallTarget,
) -> Result<PathBuf> {
    target.verify_identity()?;
    target.check_vacant()?;
    let stage_name = format!(
        ".intend-stage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    mkdirat_x(&target.parent, Path::new(&stage_name), 0o700).wrap_err_with(|| {
        format!(
            "creating staging dir {stage_name} in {}",
            target.parent_path.display()
        )
    })?;
    let staged = (|| -> Result<()> {
        use std::io::Write;
        let stage = openat_dir(&target.parent, Path::new(&stage_name))?;
        // EXCLUSIVE directory creation (review finding 6): plan.dirs is sorted
        // (parents precede children) and deduplicated, so every mkdirat must
        // succeed exactly once — an AlreadyExists here means the TARGET
        // FILESYSTEM coalesced two bytewise-distinct names (case/Unicode
        // aliasing beyond our fold, e.g. APFS ſ/s) and we fail closed instead
        // of silently merging directories.
        for dir in &plan.dirs {
            mkdirat_x(&stage, dir, 0o755).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    eyre!(
                        "directory {:?} collides with another entry ON THIS FILESYSTEM \
                         (name aliasing) — failing closed",
                        dir
                    )
                } else {
                    eyre::Report::from(e).wrap_err(format!("creating {:?}", dir))
                }
            })?;
        }
        for f in &plan.files {
            // create-new + no-follow: never follows and never overwrites.
            let mut fh = openat_create_new(&stage, &f.rel)?;
            match &f.source {
                FileSource::Raw(cid) => {
                    fh.write_all(blocks.get(cid).expect("preflight guarantees presence"))?;
                }
                FileSource::Chunks(chunks) => {
                    for chunk in chunks {
                        fh.write_all(&chunk_bytes(chunk, blocks)?)?;
                    }
                }
                FileSource::Inline(data) => fh.write_all(data)?,
            }
            // Content durable BEFORE the name publishes (power-loss ordering).
            fh.sync_all()
                .wrap_err_with(|| format!("fsync staged file {:?}", f.rel))?;
        }
        // Directory-entry durability too, children before the stage root.
        for dir in plan.dirs.iter().rev() {
            openat_dir(&stage, dir)?
                .sync_all()
                .wrap_err_with(|| format!("fsync staged dir {:?}", dir))?;
        }
        stage.sync_all().wrap_err("fsync stage root")?;
        crate::failpoint("publish-staged");
        // Fail closed if the parent PATHNAME was retargeted while staging —
        // the journaled path must still name the directory we publish into.
        target.verify_identity()?;
        renameat_noreplace(&target.parent, &stage_name, &target.name)
            .wrap_err_with(|| format!("publishing install to {}", target.path.display()))?;
        crate::failpoint("publish-after-rename");
        // The published NAME is durable before the lockfile finalizes. (If
        // THIS fails, content is published and the caller's pending journal
        // covers it; the enable finalizer re-establishes durability.)
        target
            .parent
            .sync_all()
            .wrap_err("fsync parent directory after publish")?;
        Ok(())
    })();
    match staged {
        Ok(()) => Ok(target.path.clone()),
        Err(e) => {
            // Cleanup is FD-RELATIVE (round-4 P1): resolving the parent PATH
            // again could act through a retargeted pathname — abandoning the
            // real stage or deleting through the replacement. A no-op if the
            // stage no longer exists (post-rename failures).
            if let Ok(stage_c) = std::ffi::CString::new(stage_name.as_str()) {
                let _ = remove_tree_at(&target.parent, &stage_c);
            }
            Err(e)
        }
    }
}

/// Verify the complete DAG against the externally supplied `expected_root` and
/// install it at `dest` per the PROVISIONAL §9 contract: full preflight first
/// (no writes on any validation failure), then the bound-target fd-relative
/// staged publish (`InstallTarget::bind` + `publish`). Returns the plan and
/// the CANONICAL published destination.
pub fn install(
    expected_root: Cid,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    dest: &Path,
) -> Result<(InstallPlan, PathBuf)> {
    let plan = preflight(expected_root, blocks)?;
    let target = InstallTarget::bind(dest)?;
    let published = publish(&plan, blocks, &target)?;
    Ok((plan, published))
}

// ---------- fd-relative primitives (unix) ----------

fn cstr(path: &Path) -> Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).wrap_err("path contains NUL")
}

/// Open a directory by path: O_DIRECTORY + O_NOFOLLOW on the final component.
fn open_dirfd(path: &Path) -> Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    let c = cstr(path)?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err_with(|| format!("opening directory {}", path.display()));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// Open a subdirectory RELATIVE to a held directory fd (no-follow on the final
/// component; intermediate components resolve inside our private 0700 staging
/// tree, which only this process can populate).
fn openat_dir(dirfd: &std::fs::File, rel: &Path) -> Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let c = cstr(rel)?;
    let fd = unsafe {
        libc::openat(
            dirfd.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err_with(|| format!("opening staged directory {}", rel.display()));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

fn mkdirat_x(dirfd: &std::fs::File, rel: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let c = cstr(rel).map_err(std::io::Error::other)?;
    let rc = unsafe { libc::mkdirat(dirfd.as_raw_fd(), c.as_ptr(), mode as libc::mode_t) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Create a NEW file relative to a held directory fd: O_CREAT|O_EXCL (never
/// overwrites) + O_NOFOLLOW (never follows), mode 0644.
fn openat_create_new(dirfd: &std::fs::File, rel: &Path) -> Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let c = cstr(rel)?;
    let fd = unsafe {
        libc::openat(
            dirfd.as_raw_fd(),
            c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o644 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err_with(|| format!("creating staged file {}", rel.display()));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// stat a NAME inside a held directory fd without following symlinks; Ok
/// means SOMETHING exists there (any node kind) and returns its stat.
fn fstatat_nofollow(dirfd: &std::fs::File, name: &str) -> std::io::Result<libc::stat> {
    use std::os::fd::AsRawFd;
    let c = std::ffi::CString::new(name).map_err(std::io::Error::other)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::fstatat(
            dirfd.as_raw_fd(),
            c.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(st)
}

/// Recursively delete the node `name` RELATIVE to a held directory fd —
/// never re-resolving any parent PATH (round-4 P1). Used only on our own
/// private 0700 staging trees; a missing node is success (post-rename
/// cleanup). No-follow throughout: symlinks are unlinked, never traversed.
fn remove_tree_at(dirfd: &std::fs::File, name: &std::ffi::CStr) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::fstatat(
            dirfd.as_raw_fd(),
            name.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        return if e.kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(e)
        };
    }
    if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
        // Open the subdirectory relative to the fd, list it via fdopendir on
        // a DUP (fdopendir takes ownership of the fd it is given), then
        // recurse and finally remove the emptied directory by name.
        let sub = {
            let fd = unsafe {
                libc::openat(
                    dirfd.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            use std::os::fd::FromRawFd;
            unsafe { std::fs::File::from_raw_fd(fd) }
        };
        let mut children: Vec<std::ffi::CString> = Vec::new();
        unsafe {
            let dup = libc::dup(sub.as_raw_fd());
            if dup < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let dp = libc::fdopendir(dup);
            if dp.is_null() {
                let e = std::io::Error::last_os_error();
                libc::close(dup);
                return Err(e);
            }
            loop {
                let ent = libc::readdir(dp);
                if ent.is_null() {
                    break;
                }
                let child = std::ffi::CStr::from_ptr((*ent).d_name.as_ptr());
                if child.to_bytes() == b"." || child.to_bytes() == b".." {
                    continue;
                }
                children.push(child.to_owned());
            }
            libc::closedir(dp);
        }
        for child in &children {
            remove_tree_at(&sub, child)?;
        }
        drop(sub);
        let rc = unsafe { libc::unlinkat(dirfd.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
    } else {
        let rc = unsafe { libc::unlinkat(dirfd.as_raw_fd(), name.as_ptr(), 0) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Atomic no-replace rename of two NAMES inside the SAME held directory fd —
/// the fd-relative publish step (macOS renameatx_np RENAME_EXCL / Linux
/// renameat2 RENAME_NOREPLACE).
fn renameat_noreplace(dirfd: &std::fs::File, from: &str, to: &str) -> Result<()> {
    use std::os::fd::AsRawFd;
    let from_c = std::ffi::CString::new(from).wrap_err("name contains NUL")?;
    let to_c = std::ffi::CString::new(to).wrap_err("name contains NUL")?;
    #[cfg(target_os = "macos")]
    let rc = unsafe {
        libc::renameatx_np(
            dirfd.as_raw_fd(),
            from_c.as_ptr(),
            dirfd.as_raw_fd(),
            to_c.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let rc = unsafe {
        libc::renameat2(
            dirfd.as_raw_fd(),
            from_c.as_ptr(),
            dirfd.as_raw_fd(),
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (dirfd, from_c, to_c);
        eyre::bail!("no atomic no-replace rename on this platform");
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err_with(|| format!("rename_noreplace {from} -> {to}"));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Ok(())
}

/// Atomic no-replace rename: closes the check-then-rename TOCTOU (review
/// finding 5) — if ANYTHING appears at `dst` between the preflight check and
/// the publish (even a raced empty directory, which a plain `rename` would
/// silently replace), the publish fails instead.
pub fn rename_noreplace(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let src_c = std::ffi::CString::new(src.as_os_str().as_bytes())?;
    let dst_c = std::ffi::CString::new(dst.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    let rc = unsafe { libc::renamex_np(src_c.as_ptr(), dst_c.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            src_c.as_ptr(),
            libc::AT_FDCWD,
            dst_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (src_c, dst_c);
        eyre::bail!("no atomic no-replace rename on this platform");
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err_with(|| format!("rename_noreplace {} -> {}", src.display(), dst.display()));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Ok(())
}

/// Materialize a planned file's bytes from the verified block map (used by the
/// SKILL.md policy binding).
pub fn planned_file_bytes(f: &PlannedFile, blocks: &BTreeMap<Cid, Vec<u8>>) -> Result<Vec<u8>> {
    match &f.source {
        FileSource::Raw(cid) => Ok(blocks
            .get(cid)
            .ok_or_else(|| eyre!("missing block"))?
            .clone()),
        FileSource::Chunks(chunks) => {
            let mut out = Vec::with_capacity(f.bytes as usize);
            for chunk in chunks {
                out.extend_from_slice(&chunk_bytes(chunk, blocks)?);
            }
            Ok(out)
        }
        FileSource::Inline(data) => Ok(data.clone()),
    }
}

pub fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
