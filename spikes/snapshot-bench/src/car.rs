//! Minimal CAR v1 + UnixFS(dag-pb) implementation for the Gate 2 install path.
//!
//! Scope (documented, deliberate): sha2-256 CIDv1 only; codecs raw (0x55) for file
//! leaves and dag-pb (0x70) for directory nodes — exactly the listing policy's
//! allowlist. Files are single raw blocks (≤ 1 MiB); directories nest. The builder
//! exists so the bench controls both sides; its CIDs are self-consistent but NOT
//! guaranteed byte-identical to other UnixFS builders' chunking choices — interop
//! vectors are follow-up work, recorded in the results document.
//!
//! The verifier/installer side is strict: every block hash checked, every block must
//! be reachable from the root, names sanitized, depth/size bounded, exact-byte
//! extraction, lockfile with the full identity.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use eyre::{bail, eyre, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_BLOCK_BYTES: usize = 1024 * 1024;
pub const MAX_CAR_BYTES: u64 = 64 * 1024 * 1024;
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

// ---------- varint ----------

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

fn read_uvarint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut out: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*pos).ok_or_else(|| eyre!("varint truncated"))?;
        *pos += 1;
        out |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
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

/// Directory PBNode: Links (field 2, canonical order: links before Data), then
/// UnixFS Data { Type = Directory(1) } in field 1.
pub fn encode_directory(links: &[PbLink]) -> Vec<u8> {
    let mut out = Vec::new();
    for link in links {
        let l = encode_link(link);
        pb_bytes(&mut out, 2, &l);
    }
    // UnixFS Data message: field 1 varint Type = 1 (Directory).
    let unixfs = vec![0x08, 0x01];
    pb_bytes(&mut out, 1, &unixfs);
    out
}

pub struct PbNode {
    pub links: Vec<PbLink>,
    pub unixfs_type: u64,
}

pub fn decode_pbnode(raw: &[u8]) -> Result<PbNode> {
    let mut pos = 0usize;
    let mut links = Vec::new();
    let mut unixfs_type = None;
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
            2 => links.push(decode_link(body)?),
            1 => {
                // UnixFS Data message; require field 1 (Type) varint.
                let mut p = 0usize;
                let key = read_uvarint(body, &mut p)?;
                if key >> 3 != 1 || key & 7 != 0 {
                    bail!("UnixFS Data does not start with Type");
                }
                unixfs_type = Some(read_uvarint(body, &mut p)?);
            }
            other => bail!("unexpected PBNode field {other}"),
        }
    }
    Ok(PbNode {
        links,
        unixfs_type: unixfs_type.ok_or_else(|| eyre!("PBNode lacks UnixFS Data"))?,
    })
}

fn decode_link(raw: &[u8]) -> Result<PbLink> {
    let mut pos = 0usize;
    let mut cid = None;
    let mut name = None;
    let mut tsize = 0u64;
    while pos < raw.len() {
        let key = read_uvarint(raw, &mut pos)?;
        match (key >> 3, key & 7) {
            (1, 2) => {
                let len = read_uvarint(raw, &mut pos)? as usize;
                let end = pos + len;
                cid = Some(Cid::from_bytes(
                    raw.get(pos..end).ok_or_else(|| eyre!("link truncated"))?,
                )?);
                pos = end;
            }
            (2, 2) => {
                let len = read_uvarint(raw, &mut pos)? as usize;
                let end = pos + len;
                name = Some(
                    std::str::from_utf8(raw.get(pos..end).ok_or_else(|| eyre!("link truncated"))?)
                        .wrap_err("link name utf8")?
                        .to_string(),
                );
                pos = end;
            }
            (3, 0) => tsize = read_uvarint(raw, &mut pos)?,
            (f, w) => bail!("unexpected PBLink field {f} wire {w}"),
        }
    }
    Ok(PbLink {
        cid: cid.ok_or_else(|| eyre!("PBLink lacks Hash"))?,
        name: name.ok_or_else(|| eyre!("PBLink lacks Name"))?,
        tsize,
    })
}

// ---------- CAR v1 ----------

/// dag-cbor header {"roots":[CID], "version": 1} — fixed shape, hand-encoded.
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

fn decode_car_header(raw: &[u8]) -> Result<Cid> {
    // Accept exactly the shape we emit (bench scope); anything else fails closed.
    let expect_prefix: &[u8] = &[
        0xa2, 0x65, b'r', b'o', b'o', b't', b's', 0x81, 0xd8, 0x2a, 0x58,
    ];
    if raw.len() < expect_prefix.len() + 2 || &raw[..expect_prefix.len()] != expect_prefix {
        bail!("unsupported CAR header shape");
    }
    let len = raw[expect_prefix.len()] as usize;
    let start = expect_prefix.len() + 1;
    let body = raw
        .get(start..start + len)
        .ok_or_else(|| eyre!("CAR header truncated"))?;
    if body.first() != Some(&0x00) {
        bail!("CID in header lacks identity prefix");
    }
    let cid = Cid::from_bytes(&body[1..])?;
    let rest = &raw[start + len..];
    if rest != [0x67, b'v', b'e', b'r', b's', b'i', b'o', b'n', 0x01] {
        bail!("unsupported CAR header tail (require version 1)");
    }
    Ok(cid)
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

pub fn read_car(raw: &[u8]) -> Result<(Cid, BTreeMap<Cid, Vec<u8>>)> {
    if raw.len() as u64 > MAX_CAR_BYTES {
        bail!("CAR exceeds {MAX_CAR_BYTES} byte bound");
    }
    let mut pos = 0usize;
    let hlen = read_uvarint(raw, &mut pos)? as usize;
    let header = raw
        .get(pos..pos + hlen)
        .ok_or_else(|| eyre!("CAR header truncated"))?;
    pos += hlen;
    let root = decode_car_header(header)?;
    let mut blocks = BTreeMap::new();
    while pos < raw.len() {
        let blen = read_uvarint(raw, &mut pos)? as usize;
        if !(36..=36 + MAX_BLOCK_BYTES).contains(&blen) {
            bail!("block length {blen} outside bounds");
        }
        let end = pos + blen;
        let body = raw.get(pos..end).ok_or_else(|| eyre!("block truncated"))?;
        pos = end;
        let cid = Cid::from_bytes(&body[..36])?;
        let data = &body[36..];
        // Every block hash is checked against its CID at read time.
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

// ---------- build (bench-side) ----------

/// Build a UnixFS DAG from a directory. Files become single raw blocks; directories
/// nest. Deterministic: entries sorted by name.
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
            bail!("symlinks are not supported in bench trees");
        }
        if ftype.is_dir() {
            let (cid, tsize) = build_dir(&entry.path(), blocks)?;
            links.push(PbLink { cid, name, tsize });
        } else {
            let data = std::fs::read(entry.path())?;
            if data.len() > MAX_BLOCK_BYTES {
                bail!("file {name} exceeds the single-block bound {MAX_BLOCK_BYTES}");
            }
            let cid = Cid::for_block(CODEC_RAW, &data);
            let tsize = data.len() as u64;
            blocks.insert(cid, data);
            links.push(PbLink { cid, name, tsize });
        }
    }
    let node = encode_directory(&links);
    let tsize = node.len() as u64 + links.iter().map(|l| l.tsize).sum::<u64>();
    let cid = Cid::for_block(CODEC_DAG_PB, &node);
    blocks.insert(cid, node);
    Ok((cid, tsize))
}

// ---------- verify + sanitized install (consumer-side) ----------

/// Bench lockfile. `identity` is the production binding (chain, registry, item ID,
/// on-chain status, anchor); the bench has no registry in the install path, so it
/// records `None` HONESTLY rather than placeholder strings — wiring the verified
/// registry identity in is the production CLI's scope.
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lockfile {
    pub spec: String,
    pub tree_cid: String,
    /// sha2-256 of the root block's bytes (== the Tree CID's multihash digest).
    pub root_block_sha256: String,
    pub blocks: u64,
    pub total_bytes: u64,
    pub files: Vec<LockedFile>,
    pub identity: Option<LockIdentity>,
}

pub const LOCKFILE_SPEC: &str = "intend-lock-bench/0";

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LockIdentity {
    pub chain_id: u64,
    pub registry: String,
    pub item_id: String,
    pub status: u8,
    pub anchor_block: u64,
    pub anchor_block_hash: String,
    pub anchor_state_root: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LockedFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

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

/// The fully validated result of a preflight walk — everything `install` will write,
/// decided BEFORE any filesystem mutation.
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
    pub cid: Cid,
    pub bytes: u64,
    pub sha256: String,
}

impl InstallPlan {
    pub fn locked_files(&self) -> Vec<LockedFile> {
        self.files
            .iter()
            .map(|f| LockedFile {
                path: f.rel.to_string_lossy().into_owned(),
                bytes: f.bytes,
                sha256: f.sha256.clone(),
            })
            .collect()
    }
}

/// Validate the complete DAG under `expected_root` WITHOUT touching the filesystem:
/// root must equal the externally supplied Tree CID, every CAR block must be
/// reachable (shared blocks may be referenced by any number of links — the CAR still
/// carries each block once), names sanitized, depth/file/dir counts and the
/// materialized-byte policy cap enforced as the plan grows.
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

/// Fetch a block from the untrusted map, REHASHING it on first visit: `preflight`
/// and `install` are public and take an arbitrary CID→bytes map, so the map key is
/// not evidence — only the hash of the bytes is. (`read_car` also hashes at read
/// time; this makes the consumer path safe regardless of how the map was built.)
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

fn walk_plan(
    node: Cid,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    rel: PathBuf,
    depth: usize,
    used: &mut std::collections::BTreeSet<Cid>,
    plan: &mut InstallPlan,
) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("tree exceeds depth bound {MAX_DEPTH}");
    }
    let raw = fetch_verified(node, blocks, used)?;
    let pb = decode_pbnode(raw)?;
    if pb.unixfs_type != 1 {
        bail!("expected UnixFS Directory, got type {}", pb.unixfs_type);
    }
    let mut seen = std::collections::BTreeSet::new();
    for link in pb.links {
        sanitize_name(&link.name)?;
        if !seen.insert(link.name.clone()) {
            bail!("duplicate entry name {:?}", link.name);
        }
        let child_rel = rel.join(&link.name);
        match link.cid.codec {
            CODEC_RAW => {
                let data = fetch_verified(link.cid, blocks, used)?;
                // Bounds checked BEFORE the plan grows, so they hold within a single
                // directory too (the old per-directory check was ineffective).
                if plan.files.len() >= MAX_FILES {
                    bail!("tree exceeds file bound {MAX_FILES}");
                }
                plan.total_bytes += data.len() as u64;
                if plan.total_bytes > POLICY_MAX_TREE_BYTES {
                    bail!("materialized tree exceeds the {POLICY_MAX_TREE_BYTES}-byte policy cap");
                }
                plan.files.push(PlannedFile {
                    rel: child_rel,
                    cid: link.cid,
                    bytes: data.len() as u64,
                    sha256: hex_lower(&Sha256::digest(data)),
                });
            }
            CODEC_DAG_PB => {
                if plan.dirs.len() >= MAX_FILES {
                    bail!("tree exceeds directory bound {MAX_FILES}");
                }
                plan.dirs.push(child_rel.clone());
                walk_plan(link.cid, blocks, child_rel, depth + 1, used, plan)?;
            }
            other => bail!("link codec {other:#x} outside allowlist"),
        }
    }
    Ok(())
}

/// Verify the complete DAG against the externally supplied `expected_root` and
/// install it at `dest`: full preflight first (no writes on any validation failure),
/// then staging into a fresh private sibling directory, then ONE atomic rename.
/// `dest` must not exist in any form — a preexisting file, directory, or symlink
/// (checked without following) is refused, and the parent is canonicalized first so
/// the publish target is not aliased through symlinked components. On any error the
/// staging directory is removed and `dest` is untouched.
pub fn install(
    expected_root: Cid,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    dest: &Path,
) -> Result<InstallPlan> {
    let plan = preflight(expected_root, blocks)?;
    let name = dest.file_name().ok_or_else(|| {
        eyre!(
            "destination {} has no usable final component",
            dest.display()
        )
    })?;
    let parent = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let parent = parent
        .canonicalize()
        .wrap_err_with(|| format!("destination parent {} must exist", parent.display()))?;
    let final_dest = parent.join(name);
    match std::fs::symlink_metadata(&final_dest) {
        Ok(_) => bail!(
            "destination {} already exists (file, directory, or symlink) — refusing to replace",
            final_dest.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).wrap_err("stat destination"),
    }
    let stage = parent.join(format!(
        ".intend-stage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir(&stage)
        .wrap_err_with(|| format!("creating staging dir {}", stage.display()))?;
    let staged = (|| -> Result<()> {
        use std::io::Write;
        for dir in &plan.dirs {
            std::fs::create_dir_all(stage.join(dir))?;
        }
        for f in &plan.files {
            let data = blocks.get(&f.cid).expect("preflight guarantees presence");
            let out = stage.join(&f.rel);
            // create_new: never follows and never overwrites anything.
            let mut fh = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)
                .wrap_err_with(|| format!("creating {}", out.display()))?;
            fh.write_all(data)?;
        }
        Ok(())
    })();
    match staged.and_then(|()| {
        std::fs::rename(&stage, &final_dest)
            .wrap_err_with(|| format!("publishing install to {}", final_dest.display()))
    }) {
        Ok(()) => Ok(plan),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            Err(e)
        }
    }
}

pub fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
