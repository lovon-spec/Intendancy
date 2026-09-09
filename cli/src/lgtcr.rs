//! Verified export of a Kleros **Light** Curate list (`LightGeneralizedTCR`)
//! as a Uniswap-schema token list — free to run, checkable by anyone.
//!
//! A Light list keeps no item list in storage: only `mapping(bytes32 => Item)
//! items` (item id → packed status), and the item's content is an IPFS path
//! that appears once, in the `NewItem` event. So what can and cannot be
//! proven differs from the Classic registry the `intend` CLI verifies:
//!
//! - **Membership and status are proven.** Every candidate item's status slot
//!   is read with `eth_getProof` at a header-quorum anchor and verified
//!   against the anchor's state root, the same MPT verifier `intend` uses.
//! - **Content is bound.** `itemID == keccak256(path)`, and the item JSON is
//!   fetched as IPFS blocks whose bytes must hash to the CID.
//! - **Completeness rests on logs.** The candidate set comes from `NewItem`
//!   logs, which no storage proof covers. The export therefore takes the logs
//!   from at least two independently operated RPCs and requires the sets to
//!   agree; an item both of them hide is invisible. This is stated in the
//!   provenance file, not hidden.
//!
//! Everything else (chain identity, anchoring, bounded transport) is shared
//! with `intend`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockId, Filter};
use alloy::sol;
use alloy::sol_types::SolEvent;
use eyre::{bail, eyre, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::anchor::{self, ChainPin, QuorumAnchor};
use crate::car::{self, Cid};
use crate::snapshot::{verify_account, verify_slot, AccountFields, Limits};

sol! {
    /// LightGeneralizedTCR: emitted once per item, on first submission.
    event NewItem(bytes32 indexed _itemID, string _data, bool _addedDirectly);
}

/// keccak256("") — the code hash an EOA proves; never a list.
pub const EMPTY_CODE_HASH: B256 = B256::new([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
]);

/// Item statuses of a Light list (the low byte of the item's first slot).
pub const STATUS_ABSENT: u8 = 0;
pub const STATUS_REGISTERED: u8 = 1;
pub const STATUS_REGISTRATION_REQUESTED: u8 = 2;
pub const STATUS_CLEARING_REQUESTED: u8 = 3;

/// Column labels the Kleros Tokens list declares in its MetaEvidence.
pub const COL_ADDRESS: &str = "Address";
pub const COL_NAME: &str = "Name";
pub const COL_SYMBOL: &str = "Symbol";
pub const COL_DECIMALS: &str = "Decimals";
pub const COL_LOGO: &str = "Logo";

/// Largest item file this tool materializes (an item JSON is a few KiB).
pub const MAX_ITEM_BYTES: usize = 1024 * 1024;
/// Cap on a fetched reference list (`--compare`).
pub const MAX_REFERENCE_BYTES: u64 = 32 * 1024 * 1024;

// ---------- presets ----------

/// A Light Curate list this tool knows by name.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub title: &'static str,
    pub list: Address,
    pub items_slot: u64,
    /// The list's creation block (binary-searched on code presence,
    /// 2026-09-09); no NewItem log can precede it.
    pub from_block: u64,
}

/// The four Kleros Scout registries on Gnosis. Slot 10 was verified for each
/// by reading a registered item's slot word (status byte 1).
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "address-tags",
        title: "Address Tags",
        list: alloy::primitives::address!("66260C69d03837016d88c9877e61e08Ef74C59F2"),
        items_slot: 10,
        from_block: 28_182_134,
    },
    Preset {
        name: "tokens",
        title: "Tokens",
        list: alloy::primitives::address!("eE1502e29795Ef6C2D60F8D7120596abE3baD990"),
        items_slot: 10,
        from_block: 30_214_545,
    },
    Preset {
        name: "cdn",
        title: "Contract Domain Names",
        list: alloy::primitives::address!("957A53A994860BE4750810131d9c876b2f52d6E1"),
        items_slot: 10,
        from_block: 25_810_767,
    },
    Preset {
        name: "atq",
        title: "Address Tags Query",
        list: alloy::primitives::address!("Ae6aaed5434244be3699c56E7Ebc828194F26dc3"),
        items_slot: 10,
        from_block: 33_674_138,
    },
];

pub fn preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

// ---------- the generic snapshot ----------

/// One item of a Light list as the export records it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemRecord {
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub status: u8,
    #[serde(rename = "statusName")]
    pub status_name: String,
    pub path: String,
    /// The item file's `columns`, verbatim (null when not fetched).
    pub columns: serde_json::Value,
    /// The item file's `values`, verbatim (null when not fetched).
    pub values: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// A verified snapshot of a Light list: the header says what it is and where
/// it was anchored; `items` is sorted by item id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSnapshot {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub list: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub registry: Option<String>,
    pub anchor: ProvenanceAnchor,
    #[serde(rename = "includedStatuses")]
    pub included_statuses: Vec<String>,
    pub items: Vec<ItemRecord>,
}

impl ListSnapshot {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self).wrap_err("snapshot JSON")?;
        out.push(b'\n');
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).wrap_err("not a curate-export snapshot")
    }
}

/// Which statuses an export includes.
#[derive(Debug, Clone, Copy)]
pub struct StatusFilter {
    pub pending: bool,
    pub all: bool,
}

impl StatusFilter {
    pub fn includes(&self, status: u8) -> bool {
        match status {
            STATUS_REGISTERED | STATUS_CLEARING_REQUESTED => true,
            STATUS_REGISTRATION_REQUESTED => self.pending || self.all,
            _ => self.all,
        }
    }

    pub fn names(&self) -> Vec<String> {
        (0u8..=3)
            .filter(|s| self.includes(*s))
            .map(|s| status_name(s).to_string())
            .collect()
    }
}

/// CSV rendering: itemId, status, path, then one column per label in the
/// order of the first item that carries columns (labels seen later are
/// appended). Values are stringified; embedded quotes are doubled.
pub fn snapshot_csv(snapshot: &ListSnapshot) -> String {
    let mut labels: Vec<String> = Vec::new();
    for it in &snapshot.items {
        if let Some(cols) = it.columns.as_array() {
            for c in cols {
                if let Some(l) = c.get("label").and_then(|v| v.as_str()) {
                    if !labels.iter().any(|x| x == l) {
                        labels.push(l.to_string());
                    }
                }
            }
        }
        if let Some(vals) = it.values.as_object() {
            for k in vals.keys() {
                if !labels.iter().any(|x| x == k) {
                    labels.push(k.clone());
                }
            }
        }
    }
    let esc = |s: &str| -> String {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    let mut header = vec![
        "itemId".to_string(),
        "status".to_string(),
        "path".to_string(),
    ];
    header.extend(labels.iter().cloned());
    out.push_str(&header.iter().map(|h| esc(h)).collect::<Vec<_>>().join(","));
    out.push('\n');
    for it in &snapshot.items {
        let mut row = vec![it.item_id.clone(), it.status_name.clone(), it.path.clone()];
        for l in &labels {
            let v = it
                .values
                .as_object()
                .and_then(|m| m.get(l))
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            row.push(v);
        }
        out.push_str(&row.iter().map(|c| esc(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

// ---------- offline lookup ----------

/// Filters for `lookup`; all given filters must match.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    /// A 20-byte address, matched case-insensitively against any value that is
    /// an address or a CAIP-10 "eip155:<chain>:<address>" string.
    pub address: Option<Address>,
    /// Case-insensitive substring over every value (stringified).
    pub value: Option<String>,
    /// Status number.
    pub status: Option<u8>,
}

fn value_mentions_address(v: &serde_json::Value, addr: Address) -> bool {
    match v {
        serde_json::Value::String(s) => {
            let s = s.trim();
            let candidate = s.rsplit(':').next().unwrap_or(s);
            candidate.parse::<Address>().is_ok_and(|a| a == addr)
        }
        serde_json::Value::Array(items) => items.iter().any(|i| value_mentions_address(i, addr)),
        serde_json::Value::Object(map) => map.values().any(|i| value_mentions_address(i, addr)),
        _ => false,
    }
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn lookup<'a>(snapshot: &'a ListSnapshot, q: &Lookup) -> Vec<&'a ItemRecord> {
    let needle = q.value.as_ref().map(|v| v.to_lowercase());
    snapshot
        .items
        .iter()
        .filter(|it| q.status.is_none_or(|s| it.status == s))
        .filter(|it| {
            q.address.is_none_or(|a| {
                it.values
                    .as_object()
                    .is_some_and(|m| m.values().any(|v| value_mentions_address(v, a)))
            })
        })
        .filter(|it| {
            needle.as_ref().is_none_or(|n| {
                it.values.as_object().is_some_and(|m| {
                    m.values()
                        .any(|v| value_text(v).to_lowercase().contains(n.as_str()))
                })
            })
        })
        .collect()
}

// ---------- storage layout ----------

/// Slot key of `items[itemID]`'s first word: keccak256(itemID ++ uint256(slot)).
pub fn item_slot_key(item_id: B256, items_slot: u64) -> B256 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(item_id.as_slice());
    buf[32..].copy_from_slice(&U256::from(items_slot).to_be_bytes::<32>());
    keccak256(buf)
}

/// The packed first word of a Light `Item`: `Status status` (low byte),
/// `uint128 sumDeposit`, `uint120 requestCount`.
pub fn status_from_slot(value: U256) -> u8 {
    (value & U256::from(0xffu64)).to::<u8>()
}

pub fn status_name(status: u8) -> &'static str {
    match status {
        STATUS_ABSENT => "absent",
        STATUS_REGISTERED => "registered",
        STATUS_REGISTRATION_REQUESTED => "registrationRequested",
        STATUS_CLEARING_REQUESTED => "clearingRequested",
        _ => "unknown",
    }
}

// ---------- enumeration (NewItem logs) ----------

/// The candidate set one log source reports.
#[derive(Debug, Default, Clone)]
pub struct Enumeration {
    /// itemID → item path, for every log whose path hashes to its id.
    pub items: BTreeMap<B256, String>,
    /// Raw log count (duplicates included).
    pub logs: u64,
    /// Logs whose `_data` does not hash to `_itemID` (a malformed source).
    pub mismatched: Vec<(B256, String)>,
    /// eth_getLogs calls made (windows, retries included).
    pub rpc_calls: u64,
}

fn provider_for(rpc: &str) -> Result<impl Provider + Clone> {
    crate::transport::capped_provider(rpc)
}

/// Fetch every `NewItem` log of `list` in `[from, to]` from one RPC, in
/// windows that halve on failure, and check each path against its id.
pub async fn enumerate(
    rpc: &str,
    list: Address,
    from: u64,
    to: u64,
    window: u64,
) -> Result<Enumeration> {
    if from > to {
        bail!("enumeration range is empty ({from} > {to})");
    }
    let provider = provider_for(rpc)?;
    let mut out = Enumeration::default();
    let mut window = window.max(1);
    let mut start = from;
    while start <= to {
        let end = start.saturating_add(window - 1).min(to);
        let filter = Filter::new()
            .address(list)
            .event_signature(NewItem::SIGNATURE_HASH)
            .from_block(start)
            .to_block(end);
        out.rpc_calls += 1;
        let attempt =
            anchor::with_deadline(&format!("{rpc} eth_getLogs [{start}, {end}]"), async {
                provider
                    .get_logs(&filter)
                    .await
                    .map_err(|e| eyre!("{rpc}: {e}"))
            })
            .await;
        let logs = match attempt {
            Ok(logs) => logs,
            Err(e) => {
                if window > 1000 {
                    window = shrink_window(window, &format!("{e:#}"));
                    continue;
                }
                return Err(e.wrap_err("eth_getLogs failed even at the smallest window"));
            }
        };
        for log in logs {
            let ev = log
                .log_decode::<NewItem>()
                .map_err(|e| eyre!("{rpc}: undecodable NewItem log: {e}"))?;
            out.logs += 1;
            let id = ev.inner._itemID;
            let data = ev.inner._data.clone();
            if keccak256(data.as_bytes()) != id {
                out.mismatched.push((id, data));
                continue;
            }
            out.items.entry(id).or_insert(data);
        }
        if end == to {
            break;
        }
        start = end + 1;
    }
    Ok(out)
}

/// The next log window after a failure: the limit the source names in its
/// error when it names one (PublicNode: "exceed maximum block range: 50000"),
/// otherwise half. The window only ever shrinks, and never below 1000.
pub fn shrink_window(window: u64, error: &str) -> u64 {
    let named = error
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|d| d.parse::<u64>().ok())
        .filter(|n| *n >= 1000 && *n < window)
        .max();
    match named {
        Some(n) if error.to_ascii_lowercase().contains("range") => n,
        _ => (window / 2).max(1000),
    }
}

/// Describe the difference between two candidate sets, or `None` when equal.
pub fn describe_disagreement(
    a_name: &str,
    a: &Enumeration,
    b_name: &str,
    b: &Enumeration,
) -> Option<String> {
    let a_ids: BTreeSet<&B256> = a.items.keys().collect();
    let b_ids: BTreeSet<&B256> = b.items.keys().collect();
    if a_ids == b_ids {
        return None;
    }
    let only_a: Vec<String> = a_ids
        .difference(&b_ids)
        .take(5)
        .map(|id| id.to_string())
        .collect();
    let only_b: Vec<String> = b_ids
        .difference(&a_ids)
        .take(5)
        .map(|id| id.to_string())
        .collect();
    Some(format!(
        "log sources disagree: {a_name} reports {} items, {b_name} reports {}; only in {a_name}: [{}]; only in {b_name}: [{}]",
        a_ids.len(),
        b_ids.len(),
        only_a.join(", "),
        only_b.join(", ")
    ))
}

/// Find a block at or before the list's first possible `NewItem`: scan
/// windows backwards from `anchor_block` until a window holds no log and
/// the list has no code at the window's first block.
pub async fn discover_start(
    rpc: &str,
    list: Address,
    anchor_block: u64,
    window: u64,
) -> Result<(u64, u64)> {
    let provider = provider_for(rpc)?;
    let window = window.max(1);
    let mut end = anchor_block;
    let mut calls = 0u64;
    loop {
        let start = end.saturating_sub(window - 1);
        let e = enumerate(rpc, list, start, end, window).await?;
        calls += e.rpc_calls;
        if e.logs == 0 {
            calls += 1;
            let code = anchor::with_deadline(&format!("{rpc} eth_getCode at {start}"), async {
                provider
                    .get_code_at(list)
                    .block_id(BlockId::from(start))
                    .await
                    .map_err(|e| eyre!("{rpc}: {e}"))
            })
            .await?;
            if code.is_empty() || start == 0 {
                return Ok((start, calls));
            }
        }
        if start == 0 {
            return Ok((0, calls));
        }
        end = start - 1;
    }
}

// ---------- status proofs ----------

/// Proven statuses at the anchor, plus the proven account identity.
#[derive(Debug, Clone)]
pub struct ProvenStatuses {
    pub statuses: BTreeMap<B256, u8>,
    pub code_hash: B256,
    pub storage_root: B256,
    /// eth_getProof calls made.
    pub rpc_calls: u64,
}

/// Prove `items[id]`'s first word for every id at the anchor block, verifying
/// the account against the anchor state root and every slot against the
/// account's storage root. `pinned_code_hash`, when given, must equal the
/// proven one; otherwise the proven hash is recorded and must not be the
/// empty-code hash.
pub async fn prove_statuses(
    rpc: &str,
    list: Address,
    items_slot: u64,
    pinned_code_hash: Option<B256>,
    anchor: &QuorumAnchor,
    ids: &[B256],
) -> Result<ProvenStatuses> {
    let limits = Limits::default();
    let provider = provider_for(rpc)?;
    let at = BlockId::from(anchor.block_number);
    let keys: Vec<(B256, B256)> = ids
        .iter()
        .map(|id| (item_slot_key(*id, items_slot), *id))
        .collect();
    let key_to_id: BTreeMap<B256, B256> = keys.iter().copied().collect();
    let mut statuses = BTreeMap::new();
    let mut identity: Option<(B256, B256)> = None;
    let mut rpc_calls = 0u64;
    const CHUNK: usize = 250;
    for chunk in keys.chunks(CHUNK) {
        let slots: Vec<B256> = chunk.iter().map(|(k, _)| *k).collect();
        rpc_calls += 1;
        let resp = anchor::with_deadline(
            &format!("{rpc} eth_getProof ({} slots)", slots.len()),
            async {
                provider
                    .get_proof(list, slots.clone())
                    .block_id(at)
                    .await
                    .map_err(|e| eyre!("{rpc}: {e}"))
            },
        )
        .await?;
        let fields = AccountFields {
            nonce: resp.nonce,
            balance: resp.balance,
            storage_root: resp.storage_hash,
            code_hash: resp.code_hash,
        };
        if fields.code_hash == EMPTY_CODE_HASH {
            bail!(
                "{list} has no code at block {}: not a list",
                anchor.block_number
            );
        }
        let pin = pinned_code_hash.unwrap_or(fields.code_hash);
        let storage_root = verify_account(
            anchor.state_root,
            list,
            &fields,
            &resp.account_proof,
            pin,
            &limits,
        )?;
        match identity {
            None => identity = Some((fields.code_hash, storage_root)),
            Some((c, s)) => {
                if c != fields.code_hash || s != storage_root {
                    bail!("{rpc}: account identity changed between proof batches");
                }
            }
        }
        let mut seen = 0usize;
        for sp in &resp.storage_proof {
            let key = sp.key.as_b256();
            let id = *key_to_id
                .get(&key)
                .ok_or_else(|| eyre!("{rpc}: proof for an unrequested slot {key}"))?;
            verify_slot(storage_root, key, sp.value, &sp.proof, &limits)?;
            statuses.insert(id, status_from_slot(sp.value));
            seen += 1;
        }
        if seen != slots.len() {
            bail!(
                "{rpc}: {seen} storage proofs returned for {} requested slots",
                slots.len()
            );
        }
    }
    let (code_hash, storage_root) = identity.ok_or_else(|| eyre!("no items to prove"))?;
    if statuses.len() != ids.len() {
        bail!("proved {} of {} statuses", statuses.len(), ids.len());
    }
    Ok(ProvenStatuses {
        statuses,
        code_hash,
        storage_root,
        rpc_calls,
    })
}

// ---------- content (IPFS blocks by hash) ----------

/// Parse an item path. Lists carry three forms: `/ipfs/<cid>[/segment...]`
/// (the common one), `ipfs://<cid>[/segment...]`, and, in early Address Tags
/// submissions, a bare `<cid>`.
pub fn parse_ipfs_path(path: &str) -> Result<(Cid, Vec<String>)> {
    let path = path.trim();
    let rest = path
        .strip_prefix("/ipfs/")
        .or_else(|| path.strip_prefix("ipfs://"))
        .unwrap_or(path);
    let mut parts = rest.split('/');
    let cid = Cid::parse_any(parts.next().unwrap_or_default())
        .wrap_err_with(|| format!("item path {path:?}: CID"))?;
    let segments: Vec<String> = parts
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for s in &segments {
        if s == "." || s == ".." {
            bail!("item path {path:?} has a relative segment");
        }
    }
    Ok((cid, segments))
}

/// One verified block: fetched from the gateways in order until one serves
/// bytes that hash to the CID.
async fn fetch_block(gateways: &[String], cid: Cid) -> Result<Vec<u8>> {
    let text = if cid.codec == 0x70 {
        cid.to_string_v0()?
    } else {
        cid.to_string_canonical()
    };
    let mut errors = Vec::new();
    for gw in gateways {
        let url = format!("{}/ipfs/{text}?format=raw", gw.trim_end_matches('/'));
        match crate::fetch::fetch_bounded(
            &url,
            car::MAX_BLOCK_BYTES as u64,
            Some("application/vnd.ipld.raw"),
        )
        .await
        {
            Ok(bytes) => {
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                if digest == cid.digest {
                    return Ok(bytes);
                }
                errors.push(format!("{gw}: block bytes do not hash to {text}"));
            }
            Err(e) => errors.push(format!("{gw}: {e:#}")),
        }
    }
    bail!("no gateway served block {text}: {}", errors.join("; "))
}

/// Materialize the bytes behind `cid` (+ optional path segments): a raw block,
/// a dag-pb file (inline or chunked), or a directory walked by name. Every
/// block is hash-verified; total size is bounded by `MAX_ITEM_BYTES`.
pub async fn fetch_item_bytes(
    gateways: &[String],
    cid: Cid,
    segments: &[String],
) -> Result<Vec<u8>> {
    let mut cid = cid;
    let mut segs = segments.to_vec();
    let mut depth = 0usize;
    loop {
        depth += 1;
        if depth > car::MAX_DEPTH {
            bail!("item path too deep");
        }
        let block = fetch_block(gateways, cid).await?;
        if cid.codec == 0x55 {
            if !segs.is_empty() {
                bail!("cannot walk into a raw block");
            }
            return Ok(block);
        }
        let node = car::decode_pbnode_lenient(&block)?;
        if !segs.is_empty() {
            if node.unixfs.node_type != car::UNIXFS_DIRECTORY {
                bail!("path segment {:?} under a non-directory", segs[0]);
            }
            let want = segs.remove(0);
            let link = node
                .links
                .iter()
                .find(|l| l.name == want)
                .ok_or_else(|| eyre!("no entry named {want:?}"))?;
            cid = link.cid;
            continue;
        }
        if node.unixfs.node_type != car::UNIXFS_FILE {
            bail!("item is not a file (UnixFS type {})", node.unixfs.node_type);
        }
        let mut out = node.unixfs.data.unwrap_or_default();
        for link in &node.links {
            let child = fetch_block(gateways, link.cid).await?;
            let bytes = if link.cid.codec == 0x55 {
                child
            } else {
                let leaf = car::decode_pbnode_lenient(&child)?;
                if leaf.unixfs.node_type != car::UNIXFS_FILE || !leaf.links.is_empty() {
                    bail!("multi-level chunked file (outside the supported layout)");
                }
                leaf.unixfs.data.unwrap_or_default()
            };
            out.extend_from_slice(&bytes);
            if out.len() > MAX_ITEM_BYTES {
                bail!("item file exceeds {MAX_ITEM_BYTES} bytes");
            }
        }
        return Ok(out);
    }
}

/// An item file: its `columns` (verbatim) and `values` (keyed by label).
#[derive(Debug, Clone)]
pub struct ItemFile {
    pub columns: serde_json::Value,
    pub values: serde_json::Map<String, serde_json::Value>,
}

pub async fn fetch_item_file(gateways: &[String], path: &str) -> Result<ItemFile> {
    let (cid, segments) = parse_ipfs_path(path)?;
    let bytes = fetch_item_bytes(gateways, cid, &segments).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).wrap_err("item JSON")?;
    let values = json
        .get("values")
        .and_then(|v| v.as_object())
        .ok_or_else(|| eyre!("item JSON has no `values` object"))?
        .clone();
    let columns = json
        .get("columns")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(ItemFile { columns, values })
}

// ---------- token list ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub address: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(rename = "logoURI", skip_serializing_if = "Option::is_none", default)]
    pub logo_uri: Option<String>,
}

impl Token {
    /// (chainId, lowercase address): the token-lists identity.
    pub fn key(&self) -> (u64, String) {
        (self.chain_id, self.address.to_lowercase())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenList {
    pub name: String,
    pub timestamp: String,
    pub version: Version,
    pub tokens: Vec<Token>,
}

/// A rejected item and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub reason: String,
}

/// Parse a CAIP-10 style "eip155:<chainId>:<address>" value. The address is
/// accepted in any case (list submitters do not always checksum) and
/// re-emitted EIP-55 checksummed.
pub fn parse_rich_address(raw: &str) -> Result<(u64, Address)> {
    let mut parts = raw.trim().split(':');
    let ns = parts.next().unwrap_or_default();
    if ns != "eip155" {
        bail!("address {raw:?}: namespace {ns:?} is not eip155");
    }
    let chain: u64 = parts
        .next()
        .ok_or_else(|| eyre!("address {raw:?}: no chain id"))?
        .parse()
        .map_err(|e| eyre!("address {raw:?}: chain id: {e}"))?;
    let addr_text = parts
        .next()
        .ok_or_else(|| eyre!("address {raw:?}: no address"))?;
    if parts.next().is_some() {
        bail!("address {raw:?}: too many parts");
    }
    let addr: Address = addr_text
        .parse()
        .map_err(|e| eyre!("address {raw:?}: {e}"))?;
    Ok((chain, addr))
}

fn value_string(values: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<String> {
    match values.get(key) {
        Some(serde_json::Value::String(s)) => Ok(s.trim().to_string()),
        Some(serde_json::Value::Number(n)) => Ok(n.to_string()),
        Some(other) => bail!("column {key:?} has a non-text value ({other})"),
        None => bail!("column {key:?} missing"),
    }
}

/// Build one token from an item's values; `Err` means the item is skipped.
pub fn token_from_values(
    values: &serde_json::Map<String, serde_json::Value>,
    logo_base: &str,
) -> Result<Token> {
    let (chain_id, address) = parse_rich_address(&value_string(values, COL_ADDRESS)?)?;
    let name = value_string(values, COL_NAME)?;
    let symbol = value_string(values, COL_SYMBOL)?;
    if name.is_empty() || symbol.is_empty() {
        bail!("empty name or symbol");
    }
    let decimals_text = value_string(values, COL_DECIMALS)?;
    let decimals: u64 = decimals_text
        .parse()
        .map_err(|_| eyre!("decimals {decimals_text:?} is not an integer"))?;
    if decimals > 255 {
        bail!("decimals {decimals} out of range");
    }
    let logo_uri = match values.get(COL_LOGO) {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
            let s = s.trim();
            let rest = s.strip_prefix("/ipfs/").unwrap_or(s);
            Some(format!("{logo_base}{rest}"))
        }
        _ => None,
    };
    Ok(Token {
        chain_id,
        address: address.to_checksum(None),
        name,
        symbol,
        decimals: decimals as u8,
        logo_uri,
    })
}

/// Sort tokens into their canonical order: (chainId, lowercase address).
pub fn sort_tokens(tokens: &mut [Token]) {
    tokens.sort_by_key(|a| a.key());
}

/// token-lists versioning: removal → major, addition → minor, metadata → patch.
pub fn bump_version(previous: Option<&TokenList>, next: &[Token]) -> Version {
    let Some(prev) = previous else {
        return Version {
            major: 1,
            minor: 0,
            patch: 0,
        };
    };
    let prev_map: BTreeMap<(u64, String), &Token> =
        prev.tokens.iter().map(|t| (t.key(), t)).collect();
    let next_map: BTreeMap<(u64, String), &Token> = next.iter().map(|t| (t.key(), t)).collect();
    let removed = prev_map.keys().any(|k| !next_map.contains_key(k));
    let added = next_map.keys().any(|k| !prev_map.contains_key(k));
    let changed = next_map
        .iter()
        .any(|(k, t)| prev_map.get(k).is_some_and(|p| *p != *t));
    let v = &prev.version;
    if removed {
        Version {
            major: v.major + 1,
            minor: 0,
            patch: 0,
        }
    } else if added {
        Version {
            major: v.major,
            minor: v.minor + 1,
            patch: 0,
        }
    } else if changed {
        Version {
            major: v.major,
            minor: v.minor,
            patch: v.patch + 1,
        }
    } else {
        v.clone()
    }
}

/// Serialize a token list deterministically: 2-space indent, fixed key
/// order (struct order), trailing newline.
pub fn serialize_list(list: &TokenList) -> Result<Vec<u8>> {
    let mut out = serde_json::to_vec_pretty(list).wrap_err("token list JSON")?;
    out.push(b'\n');
    Ok(out)
}

/// RFC 3339 UTC from a Unix timestamp (no fractional seconds).
pub fn rfc3339(ts: u64) -> String {
    let days = ts / 86_400;
    let secs = ts % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

// ---------- diff against a reference list ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Diff {
    #[serde(rename = "oursOnly")]
    pub ours_only: Vec<Token>,
    #[serde(rename = "theirsOnly")]
    pub theirs_only: Vec<Token>,
    pub changed: Vec<Changed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changed {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub address: String,
    pub field: String,
    pub ours: String,
    pub theirs: String,
}

/// Compare two lists by (chainId, address); metadata differences cover name,
/// symbol and decimals (logo URIs differ by scheme between exports and are
/// not compared).
pub fn diff_lists(ours: &[Token], theirs: &[Token]) -> Diff {
    let our_map: BTreeMap<(u64, String), &Token> = ours.iter().map(|t| (t.key(), t)).collect();
    let their_map: BTreeMap<(u64, String), &Token> = theirs.iter().map(|t| (t.key(), t)).collect();
    let mut d = Diff::default();
    for (k, t) in &our_map {
        match their_map.get(k) {
            None => d.ours_only.push((*t).clone()),
            Some(o) => {
                let mut push = |field: &str, a: String, b: String| {
                    if a != b {
                        d.changed.push(Changed {
                            chain_id: t.chain_id,
                            address: t.address.clone(),
                            field: field.into(),
                            ours: a,
                            theirs: b,
                        });
                    }
                };
                push("name", t.name.clone(), o.name.clone());
                push("symbol", t.symbol.clone(), o.symbol.clone());
                push("decimals", t.decimals.to_string(), o.decimals.to_string());
            }
        }
    }
    for (k, t) in &their_map {
        if !our_map.contains_key(k) {
            d.theirs_only.push((*t).clone());
        }
    }
    d
}

/// Parse a reference token list leniently (only the fields we compare).
pub fn parse_reference_list(bytes: &[u8]) -> Result<Vec<Token>> {
    #[derive(Deserialize)]
    struct Loose {
        tokens: Vec<serde_json::Value>,
    }
    let loose: Loose = serde_json::from_slice(bytes).wrap_err("reference token list JSON")?;
    let mut out = Vec::new();
    for t in loose.tokens {
        let chain_id = t.get("chainId").and_then(|v| v.as_u64());
        let address = t.get("address").and_then(|v| v.as_str());
        let (Some(chain_id), Some(address)) = (chain_id, address) else {
            continue;
        };
        let Ok(addr) = address.parse::<Address>() else {
            continue;
        };
        out.push(Token {
            chain_id,
            address: addr.to_checksum(None),
            name: t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string(),
            symbol: t
                .get("symbol")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string(),
            decimals: t
                .get("decimals")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(255) as u8,
            logo_uri: t
                .get("logoURI")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    Ok(out)
}

// ---------- provenance ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceAnchor {
    pub number: u64,
    pub hash: String,
    #[serde(rename = "stateRoot")]
    pub state_root: String,
    pub timestamp: u64,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counts {
    pub logs: u64,
    #[serde(rename = "uniqueItems")]
    pub unique_items: u64,
    #[serde(rename = "byStatus")]
    pub by_status: BTreeMap<String, u64>,
    pub included: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub genesis: String,
    pub list: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub registry: Option<String>,
    #[serde(rename = "itemsSlot")]
    pub items_slot: u64,
    #[serde(rename = "codeHash")]
    pub code_hash: String,
    pub anchor: ProvenanceAnchor,
    #[serde(rename = "anchorRpcs")]
    pub anchor_rpcs: Vec<String>,
    #[serde(rename = "logRpcs")]
    pub log_rpcs: Vec<String>,
    #[serde(rename = "providerRpc")]
    pub provider_rpc: String,
    pub gateways: Vec<String>,
    #[serde(rename = "fromBlock")]
    pub from_block: u64,
    #[serde(rename = "fromBlockSource")]
    pub from_block_source: String,
    #[serde(rename = "includedStatuses")]
    pub included_statuses: Vec<String>,
    pub counts: Counts,
    pub skipped: Vec<Skipped>,
    #[serde(rename = "outputSha256")]
    pub output_sha256: String,
    #[serde(rename = "rpcCalls")]
    pub rpc_calls: u64,
    #[serde(rename = "elapsedSeconds")]
    pub elapsed_seconds: f64,
    #[serde(rename = "toolVersion")]
    pub tool_version: String,
    #[serde(rename = "commandLine")]
    pub command_line: String,
    pub note: String,
}

// ---------- the export, end to end ----------

#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub pin: ChainPin,
    pub list: Address,
    pub registry: Option<String>,
    pub items_slot: u64,
    pub code_hash: Option<B256>,
    pub anchor_rpcs: Vec<String>,
    pub log_rpcs: Vec<String>,
    pub provider_rpc: String,
    pub gateways: Vec<String>,
    pub from_block: Option<u64>,
    pub log_window: u64,
    pub concurrency: usize,
    pub filter: StatusFilter,
}

#[derive(Debug)]
pub struct ExportOutcome {
    pub snapshot: ListSnapshot,
    pub anchor: QuorumAnchor,
    pub logs: u64,
    pub unique_items: u64,
    pub mismatched: u64,
    pub by_status: BTreeMap<String, u64>,
    pub fetch_failures: u64,
    pub code_hash: B256,
    pub from_block: u64,
    pub from_block_source: String,
    pub rpc_calls: u64,
    pub anchor_mode: String,
}

/// Every step of a verified Light-list export, fail-closed. `progress` gets
/// one line per step for the terminal.
pub async fn export_list(
    cfg: &ExportConfig,
    progress: &mut dyn FnMut(String),
) -> Result<ExportOutcome> {
    if cfg.anchor_rpcs.len() < 2 {
        bail!("at least two anchor RPC sources are required");
    }
    if cfg.log_rpcs.len() < 2 {
        bail!("at least two log RPC sources are required");
    }
    if cfg.gateways.is_empty() {
        bail!("at least one gateway is required");
    }
    let mut rpc_calls = 0u64;

    // 1. Chain identity of every source.
    let mut all: Vec<&String> = cfg.anchor_rpcs.iter().chain(cfg.log_rpcs.iter()).collect();
    all.push(&cfg.provider_rpc);
    all.sort();
    all.dedup();
    for rpc in &all {
        anchor::authenticate_source_pin(rpc, &cfg.pin).await?;
        rpc_calls += 2;
    }
    progress(format!(
        "chain identity: {} sources serve chain {} with the pinned genesis",
        all.len(),
        cfg.pin.chain_id
    ));

    // 2. Anchor (each source: identity again, finalized head, header at the agreed height).
    let quorum = anchor::finalized_quorum_rpcs(&cfg.anchor_rpcs, &cfg.pin).await?;
    rpc_calls += 4 * cfg.anchor_rpcs.len() as u64;
    if quorum.state_root == B256::ZERO {
        bail!("anchor header carries a zero state root; refusing");
    }
    let anchor_mode = "header-quorum (alpha)".to_string();
    progress(format!(
        "anchor: block {} {} ({} sources agree)",
        quorum.block_number, quorum.block_hash, quorum.sources
    ));

    // 3. Enumeration from every log source; the sets must agree.
    let (from_block, from_source) = match cfg.from_block {
        Some(b) => (b, "flag or preset".to_string()),
        None => {
            let (b, calls) = discover_start(
                &cfg.log_rpcs[0],
                cfg.list,
                quorum.block_number,
                cfg.log_window,
            )
            .await?;
            rpc_calls += calls;
            (
                b,
                format!(
                    "discovered via {} (first window with no logs and no code)",
                    cfg.log_rpcs[0]
                ),
            )
        }
    };
    progress(format!(
        "enumerating NewItem logs from block {from_block} to {}",
        quorum.block_number
    ));
    let mut enums: Vec<(String, Enumeration)> = Vec::new();
    for rpc in &cfg.log_rpcs {
        let e = enumerate(
            rpc,
            cfg.list,
            from_block,
            quorum.block_number,
            cfg.log_window,
        )
        .await?;
        rpc_calls += e.rpc_calls;
        progress(format!(
            "  {rpc}: {} logs, {} items, {} path/id mismatches, {} calls",
            e.logs,
            e.items.len(),
            e.mismatched.len(),
            e.rpc_calls
        ));
        enums.push((rpc.clone(), e));
    }
    for i in 1..enums.len() {
        if let Some(msg) = describe_disagreement(&enums[0].0, &enums[0].1, &enums[i].0, &enums[i].1)
        {
            bail!("{msg}");
        }
    }
    let enumeration = enums.remove(0).1;
    let ids: Vec<B256> = enumeration.items.keys().copied().collect();
    if ids.is_empty() {
        bail!("no items enumerated");
    }

    // 4. Status proofs at the anchor.
    let proven = prove_statuses(
        &cfg.provider_rpc,
        cfg.list,
        cfg.items_slot,
        cfg.code_hash,
        &quorum,
        &ids,
    )
    .await?;
    rpc_calls += proven.rpc_calls;
    let mut by_status: BTreeMap<String, u64> = BTreeMap::new();
    for s in proven.statuses.values() {
        *by_status.entry(status_name(*s).to_string()).or_default() += 1;
    }
    progress(format!(
        "statuses proven at block {} in {} eth_getProof calls: {:?}",
        quorum.block_number, proven.rpc_calls, by_status
    ));

    // 5. Content of every included item that has any (absent items keep only their path).
    let wanted: Vec<(B256, String)> = enumeration
        .items
        .iter()
        .filter(|(id, _)| {
            let s = proven.statuses[*id];
            cfg.filter.includes(s) && s != STATUS_ABSENT
        })
        .map(|(id, path)| (*id, path.clone()))
        .collect();
    progress(format!(
        "fetching {} item files through {} gateways",
        wanted.len(),
        cfg.gateways.len()
    ));
    let fetched = fetch_all_files(Arc::new(cfg.gateways.clone()), wanted, cfg.concurrency).await;
    let mut items = Vec::new();
    let mut fetch_failures = 0u64;
    for (id, path) in &enumeration.items {
        let status = proven.statuses[id];
        if !cfg.filter.includes(status) {
            continue;
        }
        let (columns, values, error) = match fetched.get(id) {
            Some(Ok(f)) => (
                f.columns.clone(),
                serde_json::Value::Object(f.values.clone()),
                None,
            ),
            Some(Err(e)) => {
                fetch_failures += 1;
                (
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                    Some(format!("{e:#}")),
                )
            }
            None => (serde_json::Value::Null, serde_json::Value::Null, None),
        };
        items.push(ItemRecord {
            item_id: id.to_string(),
            status,
            status_name: status_name(status).to_string(),
            path: path.clone(),
            columns,
            values,
            error,
        });
    }
    items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    let snapshot = ListSnapshot {
        chain_id: cfg.pin.chain_id,
        list: cfg.list.to_checksum(None),
        registry: cfg.registry.clone(),
        anchor: ProvenanceAnchor {
            number: quorum.block_number,
            hash: format!("{}", quorum.block_hash),
            state_root: format!("{}", quorum.state_root),
            timestamp: quorum.timestamp,
            mode: anchor_mode.clone(),
        },
        included_statuses: cfg.filter.names(),
        items,
    };
    Ok(ExportOutcome {
        snapshot,
        anchor: quorum,
        logs: enumeration.logs,
        unique_items: enumeration.items.len() as u64,
        mismatched: enumeration.mismatched.len() as u64,
        by_status,
        fetch_failures,
        code_hash: proven.code_hash,
        from_block,
        from_block_source: from_source,
        rpc_calls,
        anchor_mode,
    })
}

/// Tokens from a snapshot: every Registered item's values (plus
/// ClearingRequested with `include_clearing`) through `token_from_values`.
pub fn tokens_from_snapshot(
    snapshot: &ListSnapshot,
    include_clearing: bool,
    logo_base: &str,
) -> Result<(Vec<Token>, Vec<Skipped>)> {
    let mut inputs: ItemInputs = BTreeMap::new();
    for it in &snapshot.items {
        let id: B256 = it
            .item_id
            .parse()
            .map_err(|e| eyre!("item id {}: {e}", it.item_id))?;
        let values = it.values.as_object().cloned();
        inputs.insert(id, (it.status, values));
    }
    Ok(assemble_tokens(&inputs, include_clearing, logo_base))
}

/// itemID → (status, values when fetched).
pub type ItemInputs = BTreeMap<B256, (u8, Option<serde_json::Map<String, serde_json::Value>>)>;

/// The part of the export that is pure computation, kept apart for tests:
/// given proven statuses and fetched values, produce the sorted token set
/// and the skip list. Duplicate (chainId, address) pairs keep the item with
/// the lowest item id.
pub fn assemble_tokens(
    items: &ItemInputs,
    include_clearing: bool,
    logo_base: &str,
) -> (Vec<Token>, Vec<Skipped>) {
    let mut tokens: BTreeMap<(u64, String), (B256, Token)> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (id, (status, values)) in items {
        let included = *status == STATUS_REGISTERED
            || (include_clearing && *status == STATUS_CLEARING_REQUESTED);
        if !included {
            continue;
        }
        let Some(values) = values else {
            skipped.push(Skipped {
                item_id: id.to_string(),
                reason: "content unavailable".into(),
            });
            continue;
        };
        match token_from_values(values, logo_base) {
            Ok(t) => {
                let key = t.key();
                match tokens.get(&key) {
                    Some((other, _)) if other < id => skipped.push(Skipped {
                        item_id: id.to_string(),
                        reason: format!("duplicate of item {other}"),
                    }),
                    Some((other, _)) => {
                        let other = *other;
                        skipped.push(Skipped {
                            item_id: other.to_string(),
                            reason: format!("duplicate of item {id}"),
                        });
                        tokens.insert(key, (*id, t));
                    }
                    None => {
                        tokens.insert(key, (*id, t));
                    }
                }
            }
            Err(e) => skipped.push(Skipped {
                item_id: id.to_string(),
                reason: format!("{e:#}"),
            }),
        }
    }
    let mut out: Vec<Token> = tokens.into_values().map(|(_, t)| t).collect();
    sort_tokens(&mut out);
    skipped.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    (out, skipped)
}

/// Bounded, concurrent fetch of every included item's file.
pub async fn fetch_all_files(
    gateways: Arc<Vec<String>>,
    paths: Vec<(B256, String)>,
    concurrency: usize,
) -> BTreeMap<B256, Result<ItemFile>> {
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for (id, path) in paths {
        let gws = gateways.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let mut last = None;
            for attempt in 0..3u64 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
                }
                match fetch_item_file(&gws, &path).await {
                    Ok(v) => return (id, Ok(v)),
                    Err(e) => last = Some(e),
                }
            }
            (id, Err(last.unwrap_or_else(|| eyre!("no attempt"))))
        });
    }
    let mut out = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((id, r)) => {
                out.insert(id, r);
            }
            Err(e) => {
                out.insert(B256::ZERO, Err(eyre!("fetch task failed: {e}")));
            }
        }
    }
    out
}

/// keccak of the exact bytes a chunk of the pipeline produced — used for the
/// provenance's `tokensSha256` (sha256, matching `sha256sum`).
pub fn sha256_hex(bytes: &[u8]) -> String {
    car::hex_lower(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::b256;

    #[test]
    fn slot_key_matches_cast_keccak() {
        // cast keccak 0x6463…f51d9 ++ uint256(10)
        let id = b256!("6463b3e94e984caf8e762f0301525507ac9c63c14d6618d2e71a5b65db6f51d9");
        assert_eq!(
            item_slot_key(id, 10),
            b256!("55feed6740f70786ee505fd48d3141fca7905d282be921c14ddcac313ffa4aca")
        );
    }

    #[test]
    fn status_is_the_low_byte_of_the_packed_word() {
        let usdc: U256 = "0x0000000000000000000000000000010000000000000000000000000000000001"
            .parse()
            .unwrap();
        assert_eq!(status_from_slot(usdc), STATUS_REGISTERED);
        let clearing: U256 = "0x0000000000000000000000000000020000000000000000000000000000000003"
            .parse()
            .unwrap();
        assert_eq!(status_from_slot(clearing), STATUS_CLEARING_REQUESTED);
        assert_eq!(status_from_slot(U256::ZERO), STATUS_ABSENT);
    }

    #[test]
    fn item_id_is_the_keccak_of_the_path() {
        let path = "/ipfs/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv";
        assert_eq!(
            keccak256(path.as_bytes()),
            b256!("877d38ec4b6af4566718577c14bab28d7b442db24acd451904d976d95813dcde")
        );
    }

    #[test]
    fn rich_address_parsing() {
        let (chain, addr) =
            parse_rich_address("eip155:1:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
        assert_eq!(chain, 1);
        assert_eq!(
            addr.to_checksum(None),
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
        // lowercase is tolerated and re-checksummed
        let (_, addr2) =
            parse_rich_address("eip155:100:0xe91d153e0b41518a2ce8dd3d7944fa863463a97d").unwrap();
        assert_eq!(
            addr2.to_checksum(None),
            "0xe91D153E0b41518A2Ce8Dd3D7944Fa863463a97d"
        );
        assert!(parse_rich_address("cosmos:1:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").is_err());
        assert!(parse_rich_address("eip155:x:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").is_err());
        assert!(parse_rich_address("eip155:1:0x1234").is_err());
        assert!(
            parse_rich_address("eip155:1:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48:extra")
                .is_err()
        );
    }

    fn tok(chain: u64, addr: &str, name: &str, sym: &str, dec: u8) -> Token {
        Token {
            chain_id: chain,
            address: addr.into(),
            name: name.into(),
            symbol: sym.into(),
            decimals: dec,
            logo_uri: None,
        }
    }

    #[test]
    fn version_bump_rules() {
        let a = tok(
            1,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "A",
            "A",
            18,
        );
        let b = tok(1, "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "B", "B", 6);
        let prev = TokenList {
            name: "x".into(),
            timestamp: "t".into(),
            version: Version {
                major: 2,
                minor: 3,
                patch: 4,
            },
            tokens: vec![a.clone(), b.clone()],
        };
        assert_eq!(
            bump_version(None, std::slice::from_ref(&a)),
            Version {
                major: 1,
                minor: 0,
                patch: 0
            }
        );
        assert_eq!(
            bump_version(Some(&prev), &[a.clone(), b.clone()]),
            Version {
                major: 2,
                minor: 3,
                patch: 4
            }
        );
        assert_eq!(
            bump_version(Some(&prev), std::slice::from_ref(&a)),
            Version {
                major: 3,
                minor: 0,
                patch: 0
            }
        );
        let c = tok(1, "0xcccccccccccccccccccccccccccccccccccccccc", "C", "C", 8);
        assert_eq!(
            bump_version(Some(&prev), &[a.clone(), b.clone(), c]),
            Version {
                major: 2,
                minor: 4,
                patch: 0
            }
        );
        let mut b2 = b.clone();
        b2.name = "B renamed".into();
        assert_eq!(
            bump_version(Some(&prev), &[a.clone(), b2]),
            Version {
                major: 2,
                minor: 3,
                patch: 5
            }
        );
    }

    #[test]
    fn serialization_is_deterministic_over_input_order() {
        let mut t1 = vec![
            tok(
                100,
                "0xBBBBbbbbBBBBbbbbBBBBbbbbBBBBbbbbBBBBbbbb",
                "B",
                "B",
                6,
            ),
            tok(
                1,
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "A",
                "A",
                18,
            ),
            tok(1, "0xAAAAaaaaAAAAaaaaAAAAaaaaAAAAaaaaAAAAaaab", "C", "C", 8),
        ];
        let mut t2 = vec![t1[2].clone(), t1[0].clone(), t1[1].clone()];
        sort_tokens(&mut t1);
        sort_tokens(&mut t2);
        let l1 = TokenList {
            name: "n".into(),
            timestamp: "t".into(),
            version: Version {
                major: 1,
                minor: 0,
                patch: 0,
            },
            tokens: t1,
        };
        let l2 = TokenList {
            name: "n".into(),
            timestamp: "t".into(),
            version: Version {
                major: 1,
                minor: 0,
                patch: 0,
            },
            tokens: t2,
        };
        let b1 = serialize_list(&l1).unwrap();
        let b2 = serialize_list(&l2).unwrap();
        assert_eq!(b1, b2);
        assert!(b1.ends_with(b"\n"));
        let text = String::from_utf8(b1).unwrap();
        assert!(text.starts_with(
            "{\n  \"name\": \"n\",\n  \"timestamp\": \"t\",\n  \"version\": {\n    \"major\": 1"
        ));
        assert!(text.contains(
            "\"chainId\": 1,\n      \"address\": \"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""
        ));
        assert!(!text.contains("logoURI"));
    }

    #[test]
    fn diff_logic() {
        let a = tok(
            1,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "A",
            "A",
            18,
        );
        let b = tok(1, "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "B", "B", 6);
        let c = tok(1, "0xcccccccccccccccccccccccccccccccccccccccc", "C", "C", 8);
        let mut b_theirs = b.clone();
        b_theirs.address = b.address.to_uppercase().replace("0X", "0x");
        b_theirs.decimals = 8;
        let d = diff_lists(&[a.clone(), b.clone()], &[b_theirs, c.clone()]);
        assert_eq!(d.ours_only, vec![a]);
        assert_eq!(d.theirs_only, vec![c]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].field, "decimals");
        assert_eq!(
            (d.changed[0].ours.as_str(), d.changed[0].theirs.as_str()),
            ("6", "8")
        );
    }

    #[test]
    fn rfc3339_vectors() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_788_817_292), "2026-09-07T21:41:32Z");
        assert_eq!(rfc3339(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    #[test]
    fn token_from_values_and_skips() {
        let v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"Address":"eip155:1:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","Name":" USD Coin ","Symbol":"USDC","Decimals":"6","Logo":"/ipfs/QmLogo/usdc.png","Website":"https://circle.com"}"#,
        ).unwrap();
        let t = token_from_values(&v, "ipfs://").unwrap();
        assert_eq!(t.address, "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        assert_eq!(
            (t.name.as_str(), t.symbol.as_str(), t.decimals),
            ("USD Coin", "USDC", 6)
        );
        assert_eq!(t.logo_uri.as_deref(), Some("ipfs://QmLogo/usdc.png"));
        let mut bad = v.clone();
        bad.insert("Decimals".into(), serde_json::Value::String("300".into()));
        assert!(token_from_values(&bad, "ipfs://")
            .unwrap_err()
            .to_string()
            .contains("out of range"));
        let mut bad = v.clone();
        bad.insert(
            "Decimals".into(),
            serde_json::Value::String("eighteen".into()),
        );
        assert!(token_from_values(&bad, "ipfs://").is_err());
        let mut num = v.clone();
        num.insert("Decimals".into(), serde_json::json!(18));
        assert_eq!(token_from_values(&num, "ipfs://").unwrap().decimals, 18);
    }

    #[test]
    fn assemble_keeps_registered_dedupes_and_records_skips() {
        let mk = |addr: &str, dec: &str| -> serde_json::Map<String, serde_json::Value> {
            serde_json::from_str(&format!(r#"{{"Address":"eip155:1:{addr}","Name":"N","Symbol":"S","Decimals":"{dec}","Logo":""}}"#)).unwrap()
        };
        let id1 = b256!("0000000000000000000000000000000000000000000000000000000000000001");
        let id2 = b256!("0000000000000000000000000000000000000000000000000000000000000002");
        let id3 = b256!("0000000000000000000000000000000000000000000000000000000000000003");
        let id4 = b256!("0000000000000000000000000000000000000000000000000000000000000004");
        let id5 = b256!("0000000000000000000000000000000000000000000000000000000000000005");
        let mut items = BTreeMap::new();
        items.insert(
            id2,
            (
                STATUS_REGISTERED,
                Some(mk("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "18")),
            ),
        );
        items.insert(
            id1,
            (
                STATUS_REGISTERED,
                Some(mk("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "18")),
            ),
        ); // duplicate, lower id wins
        items.insert(
            id3,
            (
                STATUS_CLEARING_REQUESTED,
                Some(mk("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "6")),
            ),
        );
        items.insert(id4, (STATUS_REGISTERED, None));
        items.insert(
            id5,
            (
                STATUS_REGISTERED,
                Some(mk("0xcccccccccccccccccccccccccccccccccccccccc", "999")),
            ),
        );
        let (tokens, skipped) = assemble_tokens(&items, false, "ipfs://");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0].address.to_lowercase(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let reasons: Vec<&str> = skipped.iter().map(|s| s.reason.as_str()).collect();
        assert_eq!(skipped.len(), 3, "{reasons:?}");
        assert!(skipped
            .iter()
            .any(|s| s.item_id == id2.to_string() && s.reason.contains("duplicate of item")));
        assert!(skipped
            .iter()
            .any(|s| s.item_id == id4.to_string() && s.reason == "content unavailable"));
        assert!(skipped
            .iter()
            .any(|s| s.item_id == id5.to_string() && s.reason.contains("out of range")));
        let (tokens, _) = assemble_tokens(&items, true, "ipfs://");
        assert_eq!(tokens.len(), 2);
    }

    fn full(id: &str) -> String {
        format!("0x{:0>64}", id.trim_start_matches("0x"))
    }

    fn sample_snapshot() -> ListSnapshot {
        let mk = |id: &str, status: u8, values: serde_json::Value| ItemRecord {
            item_id: format!("0x{:0>64}", id.trim_start_matches("0x")),
            status,
            status_name: status_name(status).into(),
            path: format!("/ipfs/Qm{id}"),
            columns: serde_json::json!([{"label": "Address", "type": "rich address"}, {"label": "Name", "type": "text"}]),
            values,
            error: None,
        };
        ListSnapshot {
            chain_id: 100,
            list: "0x66260C69d03837016d88c9877e61e08Ef74C59F2".into(),
            registry: Some("address-tags".into()),
            anchor: ProvenanceAnchor {
                number: 1,
                hash: "0x00".into(),
                state_root: "0x00".into(),
                timestamp: 0,
                mode: "test".into(),
            },
            included_statuses: vec!["registered".into(), "clearingRequested".into()],
            items: vec![
                mk(
                    "0x02",
                    STATUS_REGISTERED,
                    serde_json::json!({"Address": "eip155:1:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "Name": "USDC, the stable one"}),
                ),
                mk(
                    "0x01",
                    STATUS_CLEARING_REQUESTED,
                    serde_json::json!({"Address": "eip155:100:0xe91d153e0b41518a2ce8dd3d7944fa863463a97d", "Name": "wxDAI \"wrapped\""}),
                ),
                mk(
                    "0x03",
                    STATUS_REGISTERED,
                    serde_json::json!({"Address": "0xA0B86991C6218B36C1D19D4A2E9EB0CE3606EB48", "Name": "plain, upper-case address"}),
                ),
            ],
        }
    }

    #[test]
    fn lookup_by_address_value_and_status() {
        let snap = sample_snapshot();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let hits = lookup(
            &snap,
            &Lookup {
                address: Some(usdc),
                ..Default::default()
            },
        );
        let ids: Vec<&str> = hits.iter().map(|h| h.item_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![full("0x02"), full("0x03")],
            "rich-address and plain-address values both match, case-insensitively"
        );
        let hits = lookup(
            &snap,
            &Lookup {
                value: Some("WRAPPED".into()),
                ..Default::default()
            },
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_id, full("0x01"));
        let hits = lookup(
            &snap,
            &Lookup {
                status: Some(STATUS_REGISTERED),
                value: Some("address".into()),
                ..Default::default()
            },
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_id, full("0x03"));
        assert!(lookup(
            &snap,
            &Lookup {
                status: Some(STATUS_ABSENT),
                ..Default::default()
            }
        )
        .is_empty());
    }

    #[test]
    fn csv_flattens_values_by_label_and_escapes() {
        let csv = snapshot_csv(&sample_snapshot());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "itemId,status,path,Address,Name");
        assert_eq!(lines.len(), 4);
        assert!(
            lines.iter().any(|l| l.contains("\"USDC, the stable one\"")),
            "commas are quoted: {csv}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("\"wxDAI \"\"wrapped\"\"\"")),
            "quotes are doubled: {csv}"
        );
    }

    #[test]
    fn snapshot_round_trips_and_status_filter() {
        let snap = sample_snapshot();
        let bytes = snap.to_bytes().unwrap();
        assert!(bytes.ends_with(b"\n"));
        let back = ListSnapshot::from_bytes(&bytes).unwrap();
        assert_eq!(back.items, snap.items);
        assert_eq!(back.registry.as_deref(), Some("address-tags"));
        let f = StatusFilter {
            pending: false,
            all: false,
        };
        assert!(f.includes(STATUS_REGISTERED) && f.includes(STATUS_CLEARING_REQUESTED));
        assert!(!f.includes(STATUS_REGISTRATION_REQUESTED) && !f.includes(STATUS_ABSENT));
        let f = StatusFilter {
            pending: true,
            all: false,
        };
        assert!(f.includes(STATUS_REGISTRATION_REQUESTED) && !f.includes(STATUS_ABSENT));
        assert_eq!(
            f.names(),
            vec!["registered", "registrationRequested", "clearingRequested"]
        );
        assert!(StatusFilter {
            pending: false,
            all: true
        }
        .includes(STATUS_ABSENT));
    }

    #[test]
    fn presets_are_distinct_and_named() {
        let names: BTreeSet<&str> = PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), PRESETS.len());
        let lists: BTreeSet<Address> = PRESETS.iter().map(|p| p.list).collect();
        assert_eq!(lists.len(), PRESETS.len());
        assert!(PRESETS
            .iter()
            .all(|p| p.items_slot == 10 && p.from_block > 0));
        assert_eq!(
            preset("tokens").unwrap().list.to_checksum(None),
            "0xeE1502e29795Ef6C2D60F8D7120596abE3baD990"
        );
        assert!(preset("nope").is_none());
    }

    #[test]
    fn tokens_from_a_snapshot() {
        let mut snap = sample_snapshot();
        for it in &mut snap.items {
            if let Some(m) = it.values.as_object_mut() {
                m.insert("Symbol".into(), serde_json::json!("S"));
                m.insert("Decimals".into(), serde_json::json!("6"));
            }
        }
        let (tokens, skipped) = tokens_from_snapshot(&snap, false, "ipfs://").unwrap();
        // 0x02 registered (USDC), 0x03 registered (plain address, no eip155 prefix → skipped), 0x01 clearing → excluded
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0].address,
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("eip155"));
        let (tokens, _) = tokens_from_snapshot(&snap, true, "ipfs://").unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn log_window_shrinks_to_a_named_limit_or_halves() {
        assert_eq!(shrink_window(1_000_000, "server returned an error response: error code -32701: exceed maximum block range: 50000"), 50_000);
        assert_eq!(
            shrink_window(1_000_000, "deadline of 30s exceeded"),
            500_000
        );
        assert_eq!(
            shrink_window(1_000_000, "query returned more than 10000 results"),
            500_000
        );
        assert_eq!(
            shrink_window(40_000, "exceed maximum block range: 50000"),
            20_000
        );
        assert_eq!(shrink_window(1500, "anything"), 1000);
    }

    #[test]
    fn ipfs_path_parsing() {
        let (cid, segs) =
            parse_ipfs_path("/ipfs/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv").unwrap();
        assert_eq!(cid.codec, 0x70);
        assert_eq!(
            car::hex_lower(&cid.digest),
            "7f218c635c3a4d09afb01fab06272cb69a139318cc9c92f8062dd8f896284a6b"
        );
        assert!(segs.is_empty());
        let (_, segs) =
            parse_ipfs_path("/ipfs/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv/item.json")
                .unwrap();
        assert_eq!(segs, vec!["item.json".to_string()]);
        // Early Address Tags submissions carry a bare CID; ipfs:// also occurs.
        let (bare, segs) =
            parse_ipfs_path("QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv").unwrap();
        assert_eq!((bare, segs.len()), (cid, 0));
        let (scheme, _) =
            parse_ipfs_path("ipfs://QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv/item.json")
                .unwrap();
        assert_eq!(scheme, cid);
        assert!(parse_ipfs_path(
            "https://example.invalid/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv"
        )
        .is_err());
        assert!(
            parse_ipfs_path("/ipfs/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv/../x").is_err()
        );
    }
}
