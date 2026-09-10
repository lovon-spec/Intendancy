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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockId, Filter};
use alloy::sol;
use alloy::sol_types::SolEvent;
use eyre::{bail, eyre, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use crate::anchor::{self, ChainPin, QuorumAnchor, Source};
use crate::car::{self, Cid};
use crate::snapshot::{verify_account, verify_slot, AccountFields, Limits};
use crate::transport::{RpcCounts, RpcStats};

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
    /// Calls repeated at the same window after a transient failure.
    pub retries: u64,
    /// Window reductions after a range or result-size limit.
    pub shrinks: u64,
    /// Window doublings after sustained success.
    pub grows: u64,
}

/// No window shrinks below this many blocks.
pub const LOG_MIN_WINDOW: u64 = 1000;
/// Same-window retries of a transient failure before the scan gives up.
pub const LOG_TRANSIENT_RETRIES: u32 = 3;
/// Backoff before the first such retry; it doubles per retry.
pub const LOG_RETRY_BACKOFF: Duration = Duration::from_secs(1);
/// Consecutive successful windows before a shrunken window doubles again.
pub const LOG_GROW_AFTER: u32 = 8;

/// How the log scan sizes its windows and treats failures.
#[derive(Debug, Clone, Copy)]
pub struct LogWindowPolicy {
    /// The configured window, and the ceiling growth returns to.
    pub window: u64,
    pub min_window: u64,
    pub transient_retries: u32,
    pub backoff: Duration,
    pub grow_after: u32,
}

impl LogWindowPolicy {
    pub fn new(window: u64) -> Self {
        Self {
            window: window.max(1),
            min_window: LOG_MIN_WINDOW,
            transient_retries: LOG_TRANSIENT_RETRIES,
            backoff: LOG_RETRY_BACKOFF,
            grow_after: LOG_GROW_AFTER,
        }
    }
}

/// What a failed `eth_getLogs` call says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogError {
    /// The source refuses the range or the result size; `named` is the limit
    /// it states, when it states one in a recognised form.
    RangeLimit { named: Option<u64> },
    /// The request did not complete in time.
    Timeout,
    /// Anything else (a rate limit, a 5xx, a dropped connection).
    Transient,
}

/// Classify an `eth_getLogs` failure from its message. Recognised limit
/// formats name the limit: `exceed maximum block range: 50000` (PublicNode),
/// `limited to a 10,000 blocks range` (QuickNode), `up to a 2K block range`
/// (Alchemy), `max 1000 blocks`, and a suggested range `[0x…, 0x…]` (Infura,
/// Alchemy), whose size is the limit. Other messages about the range or the
/// result size are limits without a number; a number elsewhere in the
/// message (a request id, a block number) is never taken for the limit.
pub fn classify_log_error(message: &str) -> LogError {
    let m = message.to_ascii_lowercase();
    if let Some(n) = suggested_range(&m).or_else(|| named_limit(&m)) {
        return LogError::RangeLimit { named: Some(n) };
    }
    const LIMIT_PHRASES: &[&str] = &[
        "block range",
        "range too large",
        "range is too large",
        "range too wide",
        "range is too wide",
        "range too big",
        "too many results",
        "returned more than",
        "response size exceeded",
        "exceeds max results",
        "exceed max results",
        "result set too large",
        "too many logs",
        "log limit",
        "logs limit",
    ];
    if LIMIT_PHRASES.iter().any(|p| m.contains(p)) {
        return LogError::RangeLimit { named: None };
    }
    const TIMEOUT_PHRASES: &[&str] = &[
        "deadline",
        "timed out",
        "timeout",
        "time out",
        "http 408",
        "http 504",
        "gateway time",
    ];
    if TIMEOUT_PHRASES.iter().any(|p| m.contains(p)) {
        return LogError::Timeout;
    }
    LogError::Transient
}

/// The size of a suggested `[0x…, 0x…]` block range in the message.
fn suggested_range(m: &str) -> Option<u64> {
    let open = m.find("[0x")?;
    let close = m[open..].find(']')? + open;
    let inner = &m[open + 1..close];
    let (a, b) = inner.split_once(',')?;
    let parse = |s: &str| u64::from_str_radix(s.trim().strip_prefix("0x")?, 16).ok();
    let (a, b) = (parse(a)?, parse(b)?);
    (b >= a).then_some(b - a + 1)
}

/// A limit stated next to the words `range`, `block` or `blocks`, in a
/// message that speaks of a limit at all.
fn named_limit(m: &str) -> Option<u64> {
    // "10,000" is one number; "2k" is two thousand.
    let mut cleaned = String::with_capacity(m.len());
    let chars: Vec<char> = m.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let between_digits = *c == ','
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if !between_digits {
            cleaned.push(*c);
        }
    }
    let tokens: Vec<&str> = cleaned
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    const LIMIT_WORDS: &[&str] = &[
        "max", "maximum", "limit", "limited", "exceed", "exceeds", "exceeded", "allowed", "up",
        "large", "big", "wide",
    ];
    if !tokens.iter().any(|t| LIMIT_WORDS.contains(t)) {
        return None;
    }
    let count = |t: &str| -> Option<u64> {
        let (digits, mult) = match t.strip_suffix('k') {
            Some(d) => (d, 1000),
            None => (t, 1),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits
            .parse::<u64>()
            .ok()?
            .checked_mul(mult)
            .filter(|n| *n > 0)
    };
    for (i, t) in tokens.iter().enumerate() {
        let Some(n) = count(t) else { continue };
        let is = |j: usize, words: &[&str]| tokens.get(j).is_some_and(|w| words.contains(w));
        let after_range = (i >= 1 && is(i - 1, &["range"]))
            || (i >= 2 && is(i - 2, &["range"]) && is(i - 1, &["of", "is", "to", "at", "allowed"]));
        let before_blocks = is(i + 1, &["block", "blocks"]);
        if after_range || before_blocks {
            return Some(n);
        }
    }
    None
}

/// The window to scan next after a limit error, or `None` when the scan is
/// already at the smallest window: the named limit when it is smaller than
/// the window, otherwise half, never below `min_window`.
pub fn next_window(window: u64, error: &LogError, min_window: u64) -> Option<u64> {
    let LogError::RangeLimit { named } = error else {
        return Some(window);
    };
    if window <= min_window {
        return None;
    }
    let next = named
        .filter(|n| *n < window)
        .unwrap_or(window / 2)
        .max(min_window);
    Some(next)
}

/// Fetch every `NewItem` log of `list` in `[from, to]` from one RPC and
/// check each path against its id, under the default window policy.
pub async fn enumerate(
    source: &Source,
    list: Address,
    from: u64,
    to: u64,
    window: u64,
) -> Result<Enumeration> {
    enumerate_with(source, list, from, to, &LogWindowPolicy::new(window)).await
}

/// `enumerate` under an explicit policy: a recognised range or result-size
/// limit shrinks the window (to the named limit when there is one, otherwise
/// by half, never below the minimum); a transient failure is retried at the
/// same window with doubling backoff; a window that keeps timing out is
/// treated as too large; and a shrunken window doubles again after
/// sustained success, up to the configured window or the limit a source
/// named.
pub async fn enumerate_with(
    source: &Source,
    list: Address,
    from: u64,
    to: u64,
    policy: &LogWindowPolicy,
) -> Result<Enumeration> {
    if from > to {
        bail!("enumeration range is empty ({from} > {to})");
    }
    let rpc = source.rpc.as_str();
    let provider = source.provider();
    let mut out = Enumeration::default();
    let mut ceiling = policy.window.max(1);
    let min_window = policy.min_window.max(1).min(ceiling);
    let mut window = ceiling;
    let mut streak = 0u32;
    let mut failures_here = 0u32;
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
                let kind = classify_log_error(&format!("{e:#}"));
                match kind {
                    LogError::RangeLimit { named } => {
                        let Some(next) = next_window(window, &kind, min_window) else {
                            return Err(
                                e.wrap_err("eth_getLogs failed even at the smallest window")
                            );
                        };
                        if named.is_some_and(|n| n < window) {
                            ceiling = ceiling.min(next);
                        }
                        window = next;
                        streak = 0;
                        failures_here = 0;
                        out.shrinks += 1;
                        continue;
                    }
                    LogError::Timeout | LogError::Transient => {
                        failures_here += 1;
                        if failures_here <= policy.transient_retries {
                            out.retries += 1;
                            tokio::time::sleep(policy.backoff * 2u32.pow(failures_here - 1)).await;
                            continue;
                        }
                        if kind == LogError::Timeout && window > min_window {
                            // Timing out every time at this size: too large for
                            // this source; halve, and let success grow it back.
                            window = (window / 2).max(min_window);
                            streak = 0;
                            failures_here = 0;
                            out.shrinks += 1;
                            continue;
                        }
                        return Err(e.wrap_err(format!(
                            "eth_getLogs failed {failures_here} times at [{start}, {end}]"
                        )));
                    }
                }
            }
        };
        failures_here = 0;
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
        streak += 1;
        if window < ceiling && streak >= policy.grow_after {
            window = window.saturating_mul(2).min(ceiling);
            streak = 0;
            out.grows += 1;
        }
        if end == to {
            break;
        }
        start = end + 1;
    }
    Ok(out)
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
    source: &Source,
    list: Address,
    anchor_block: u64,
    window: u64,
) -> Result<(u64, u64)> {
    let rpc = source.rpc.as_str();
    let provider = source.provider();
    let window = window.max(1);
    let mut end = anchor_block;
    let mut calls = 0u64;
    loop {
        let start = end.saturating_sub(window - 1);
        let e = enumerate(source, list, start, end, window).await?;
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
    source: &Source,
    list: Address,
    items_slot: u64,
    pinned_code_hash: Option<B256>,
    anchor: &QuorumAnchor,
    ids: &[B256],
) -> Result<ProvenStatuses> {
    let limits = Limits::default();
    let rpc = source.rpc.as_str();
    let provider = source.provider();
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

/// Request deadline for one block: item files are a few KiB.
pub const BLOCK_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-item work budget (see `ItemBudget`). A root block under the block cap
/// can name thousands of links, so the size of the output bounds nothing;
/// these do.
pub const ITEM_MAX_BLOCKS: usize = 256;
pub const ITEM_MAX_ATTEMPTS: u64 = 64;
pub const ITEM_MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
pub const ITEM_TIME_BUDGET: Duration = Duration::from_secs(120);
/// Rounds an item gets while a block is unavailable (each round asks every
/// gateway once); round `n` waits `ITEM_RETRY_BACKOFF × n` first, in the
/// delayed queue rather than as an admitted task.
pub const ITEM_MAX_ROUNDS: u32 = 3;
pub const ITEM_RETRY_BACKOFF: Duration = Duration::from_millis(500);
/// Bounds of the run's verified-block cache.
pub const BLOCK_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const BLOCK_CACHE_MAX_ENTRIES: usize = 65_536;

/// What one item may cost, whatever its shape: blocks visited (links
/// followed, cache hits included), network attempts, bytes downloaded (bytes
/// that fail their hash included) and wall-clock time from its first
/// admission, retry rounds included. Exceeding any of them is final.
#[derive(Debug, Clone, Copy)]
pub struct ItemBudget {
    pub max_blocks: usize,
    pub max_attempts: u64,
    pub max_download_bytes: u64,
    pub time: Duration,
    /// Deadline of one block request, shortened to what is left of `time`.
    pub block_timeout: Duration,
    pub max_rounds: u32,
    pub retry_backoff: Duration,
}

impl Default for ItemBudget {
    fn default() -> Self {
        Self {
            max_blocks: ITEM_MAX_BLOCKS,
            max_attempts: ITEM_MAX_ATTEMPTS,
            max_download_bytes: ITEM_MAX_DOWNLOAD_BYTES,
            time: ITEM_TIME_BUDGET,
            block_timeout: BLOCK_FETCH_TIMEOUT,
            max_rounds: ITEM_MAX_ROUNDS,
            retry_backoff: ITEM_RETRY_BACKOFF,
        }
    }
}

/// One item's spent budget, carried across its rounds.
#[derive(Debug, Clone)]
pub struct ItemWork {
    pub deadline: Instant,
    /// Blocks visited in the current round (a retry replays the walk, through
    /// the cache for the blocks it already verified).
    pub blocks: usize,
    pub attempts: u64,
    pub downloaded: u64,
    pub rounds: u32,
}

impl ItemWork {
    pub fn new(budget: &ItemBudget) -> Self {
        Self {
            deadline: Instant::now() + budget.time,
            blocks: 0,
            attempts: 0,
            downloaded: 0,
            rounds: 0,
        }
    }
}

/// Why an item's content could not be produced. Transient: no gateway served
/// a block in this round (availability; a later round may). Permanent:
/// hash-verified bytes that are not an item file, an unsupported layout, a bad
/// path, an exhausted budget — fetching the same bytes again changes nothing,
/// so nothing is retried.
#[derive(Debug)]
pub enum FetchError {
    Transient(eyre::Report),
    Permanent(eyre::Report),
}

impl FetchError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    pub fn report(&self) -> &eyre::Report {
        match self {
            Self::Transient(r) | Self::Permanent(r) => r,
        }
    }

    pub fn into_report(self) -> eyre::Report {
        match self {
            Self::Transient(r) | Self::Permanent(r) => r,
        }
    }

    fn permanent(e: impl Into<eyre::Report>) -> Self {
        Self::Permanent(e.into())
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.report())
    }
}

/// Gateways with a per-run health score: a failure adds a penalty, a success
/// clears it, and every block tries the gateways in order of least penalty, so
/// a stalled gateway stops costing every item its timeout.
#[derive(Debug, Default)]
pub struct Gateways {
    urls: Vec<String>,
    penalties: std::sync::Mutex<Vec<u32>>,
}

impl Gateways {
    pub fn new(urls: &[String]) -> Self {
        Self {
            urls: urls.to_vec(),
            penalties: std::sync::Mutex::new(vec![0; urls.len()]),
        }
    }

    pub fn ordered(&self) -> Vec<usize> {
        let p = self.penalties.lock().unwrap_or_else(|e| e.into_inner());
        let mut idx: Vec<usize> = (0..self.urls.len()).collect();
        idx.sort_by_key(|i| (p[*i], *i));
        idx
    }

    fn report(&self, i: usize, ok: bool) {
        let mut p = self.penalties.lock().unwrap_or_else(|e| e.into_inner());
        p[i] = if ok { 0 } else { p[i].saturating_add(1) };
    }

    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }
}

/// Run-scoped counters of the content pipeline.
#[derive(Debug, Default)]
pub struct IpfsStats {
    requests: AtomicU64,
    response_bytes: AtomicU64,
    bad_bytes: AtomicU64,
    cache_hits: AtomicU64,
    coalesced: AtomicU64,
    item_retries: AtomicU64,
}

/// A reading of `IpfsStats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpfsCounts {
    /// Block requests sent to gateways (every attempt at every gateway).
    pub requests: u64,
    /// Response bytes downloaded, bytes that failed their hash included.
    #[serde(rename = "responseBytes")]
    pub response_bytes: u64,
    /// Responses whose bytes did not hash to the CID asked for.
    #[serde(rename = "badBytes")]
    pub bad_bytes: u64,
    /// Blocks served from the run's verified-block cache.
    #[serde(rename = "cacheHits")]
    pub cache_hits: u64,
    /// Block requests that waited for another task's fetch of the same block.
    pub coalesced: u64,
    /// Item rounds beyond the first (a block was unavailable).
    #[serde(rename = "itemRetries")]
    pub item_retries: u64,
}

impl IpfsStats {
    pub fn counts(&self) -> IpfsCounts {
        IpfsCounts {
            requests: self.requests.load(Ordering::Relaxed),
            response_bytes: self.response_bytes.load(Ordering::Relaxed),
            bad_bytes: self.bad_bytes.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            item_retries: self.item_retries.load(Ordering::Relaxed),
        }
    }
}

/// Hash-verified blocks of this run, keyed by CID (codec and digest: the same
/// bytes under a CIDv0 and a CIDv1 text form are one entry), bounded in bytes
/// and entries with oldest-first eviction. A miss in flight is shared: every
/// concurrent caller for one CID waits for the one fetch. A failed fetch
/// leaves no entry, so an availability failure is never remembered.
struct BlockCache {
    inner: std::sync::Mutex<CacheInner>,
    max_bytes: usize,
    max_entries: usize,
}

struct CacheInner {
    entries: BTreeMap<Cid, Arc<tokio::sync::OnceCell<Arc<[u8]>>>>,
    order: VecDeque<Cid>,
    bytes: usize,
}

impl BlockCache {
    fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(CacheInner {
                entries: BTreeMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            }),
            max_bytes,
            max_entries: max_entries.max(1),
        }
    }

    async fn get_or_fetch<F, Fut>(
        &self,
        cid: Cid,
        stats: &IpfsStats,
        fetch: F,
    ) -> Result<Arc<[u8]>, FetchError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, FetchError>>,
    {
        let (cell, known) = {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match g.entries.get(&cid) {
                Some(c) => (c.clone(), true),
                None => {
                    let c = Arc::new(tokio::sync::OnceCell::new());
                    g.entries.insert(cid, c.clone());
                    g.order.push_back(cid);
                    (c, false)
                }
            }
        };
        let ready = cell.initialized();
        let mut fetched_here = false;
        let bytes = cell
            .get_or_try_init(|| {
                fetched_here = true;
                async move { fetch().await.map(Arc::<[u8]>::from) }
            })
            .await?
            .clone();
        if ready {
            stats.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else if known && !fetched_here {
            stats.coalesced.fetch_add(1, Ordering::Relaxed);
        }
        if fetched_here {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.entries.get(&cid).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                g.bytes += bytes.len();
            }
            while (g.bytes > self.max_bytes || g.entries.len() > self.max_entries)
                && g.order.len() > 1
            {
                let Some(old) = g.order.pop_front() else {
                    break;
                };
                if let Some(c) = g.entries.remove(&old) {
                    if let Some(b) = c.get() {
                        g.bytes = g.bytes.saturating_sub(b.len());
                    }
                }
            }
        }
        Ok(bytes)
    }
}

/// The content pipeline of one run: gateways ordered by health, the
/// verified-block cache, the network permits (`concurrency` block requests in
/// flight; a permit spans exactly one blocking request and nothing else), the
/// per-item budget and the counters.
pub struct Fetcher {
    gateways: Gateways,
    cache: BlockCache,
    network: tokio::sync::Semaphore,
    budget: ItemBudget,
    stats: IpfsStats,
}

impl Fetcher {
    pub fn new(gateways: &[String], concurrency: usize) -> Self {
        Self {
            gateways: Gateways::new(gateways),
            cache: BlockCache::new(BLOCK_CACHE_MAX_BYTES, BLOCK_CACHE_MAX_ENTRIES),
            network: tokio::sync::Semaphore::new(concurrency.max(1)),
            budget: ItemBudget::default(),
            stats: IpfsStats::default(),
        }
    }

    pub fn with_budget(mut self, budget: ItemBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn budget(&self) -> &ItemBudget {
        &self.budget
    }

    pub fn gateways(&self) -> &Gateways {
        &self.gateways
    }

    pub fn counts(&self) -> IpfsCounts {
        self.stats.counts()
    }

    /// One verified block, from the cache or from the gateways, counted
    /// against the item's block budget either way.
    async fn fetch_block(&self, cid: Cid, work: &mut ItemWork) -> Result<Arc<[u8]>, FetchError> {
        work.blocks += 1;
        if work.blocks > self.budget.max_blocks {
            return Err(FetchError::Permanent(eyre!(
                "item traverses more than {} blocks",
                self.budget.max_blocks
            )));
        }
        self.cache
            .get_or_fetch(cid, &self.stats, move || {
                self.fetch_block_network(cid, work)
            })
            .await
    }

    /// One block from the gateways, healthiest first, until one serves bytes
    /// that hash to the CID. A gateway's wrong bytes or failure moves on to
    /// the next; when none served the block this round, the failure is
    /// transient. Every request's deadline is the shorter of the block
    /// timeout and what is left of the item's time budget, and the network
    /// permit is held until the blocking request has returned (a dropped
    /// future would not stop it).
    async fn fetch_block_network(
        &self,
        cid: Cid,
        work: &mut ItemWork,
    ) -> Result<Vec<u8>, FetchError> {
        let text = if cid.codec == 0x70 {
            cid.to_string_v0().map_err(FetchError::permanent)?
        } else {
            cid.to_string_canonical()
        };
        let mut errors = Vec::new();
        for i in self.gateways.ordered() {
            if work.attempts >= self.budget.max_attempts {
                return Err(FetchError::Permanent(eyre!(
                    "item exhausted its network budget of {} attempts (block {text}: {})",
                    self.budget.max_attempts,
                    errors.join("; ")
                )));
            }
            let remaining = work.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FetchError::Permanent(eyre!(
                    "item exhausted its time budget of {:?} (block {text}: {})",
                    self.budget.time,
                    errors.join("; ")
                )));
            }
            let timeout = remaining.min(self.budget.block_timeout);
            work.attempts += 1;
            self.stats.requests.fetch_add(1, Ordering::Relaxed);
            let gw = &self.gateways.urls[i];
            let url = format!("{}/ipfs/{text}?format=raw", gw.trim_end_matches('/'));
            let permit = self
                .network
                .acquire()
                .await
                .map_err(|_| FetchError::Permanent(eyre!("fetcher closed")))?;
            let res = crate::fetch::fetch_bounded_timeout(
                &url,
                car::MAX_BLOCK_BYTES as u64,
                Some("application/vnd.ipld.raw"),
                timeout,
            )
            .await;
            drop(permit);
            match res {
                Ok(bytes) => {
                    work.downloaded += bytes.len() as u64;
                    self.stats
                        .response_bytes
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    let over_budget = work.downloaded > self.budget.max_download_bytes;
                    let digest: [u8; 32] = Sha256::digest(&bytes).into();
                    if digest == cid.digest {
                        self.gateways.report(i, true);
                        if over_budget {
                            return Err(FetchError::Permanent(eyre!(
                                "item exhausted its download budget of {} bytes",
                                self.budget.max_download_bytes
                            )));
                        }
                        return Ok(bytes);
                    }
                    self.gateways.report(i, false);
                    self.stats.bad_bytes.fetch_add(1, Ordering::Relaxed);
                    errors.push(format!("{gw}: block bytes do not hash to {text}"));
                    if over_budget {
                        return Err(FetchError::Permanent(eyre!(
                            "item exhausted its download budget of {} bytes ({})",
                            self.budget.max_download_bytes,
                            errors.join("; ")
                        )));
                    }
                }
                Err(e) => {
                    self.gateways.report(i, false);
                    errors.push(format!("{gw}: {e:#}"));
                }
            }
        }
        Err(FetchError::Transient(eyre!(
            "no gateway served block {text}: {}",
            errors.join("; ")
        )))
    }

    /// Materialize the bytes behind `cid` (+ optional path segments): a raw
    /// block, a dag-pb file (inline or chunked), or a directory walked by
    /// name. Every block is hash-verified; the assembled size is bounded by
    /// `MAX_ITEM_BYTES` and the work by the item's budget.
    pub async fn fetch_item_bytes(
        &self,
        cid: Cid,
        segments: &[String],
        work: &mut ItemWork,
    ) -> Result<Vec<u8>, FetchError> {
        let mut cid = cid;
        let mut segs = segments.to_vec();
        let mut depth = 0usize;
        loop {
            depth += 1;
            if depth > car::MAX_DEPTH {
                return Err(FetchError::Permanent(eyre!("item path too deep")));
            }
            let block = self.fetch_block(cid, work).await?;
            if cid.codec == 0x55 {
                if !segs.is_empty() {
                    return Err(FetchError::Permanent(eyre!("cannot walk into a raw block")));
                }
                return Ok(block.to_vec());
            }
            let node = car::decode_pbnode_lenient(&block).map_err(FetchError::permanent)?;
            if !segs.is_empty() {
                if node.unixfs.node_type != car::UNIXFS_DIRECTORY {
                    return Err(FetchError::Permanent(eyre!(
                        "path segment {:?} under a non-directory",
                        segs[0]
                    )));
                }
                let want = segs.remove(0);
                let link = node
                    .links
                    .iter()
                    .find(|l| l.name == want)
                    .ok_or_else(|| FetchError::Permanent(eyre!("no entry named {want:?}")))?;
                cid = link.cid;
                continue;
            }
            if node.unixfs.node_type != car::UNIXFS_FILE {
                return Err(FetchError::Permanent(eyre!(
                    "item is not a file (UnixFS type {})",
                    node.unixfs.node_type
                )));
            }
            let mut out = node.unixfs.data.unwrap_or_default();
            if out.len() > MAX_ITEM_BYTES {
                return Err(FetchError::Permanent(eyre!(
                    "item file exceeds {MAX_ITEM_BYTES} bytes"
                )));
            }
            for link in &node.links {
                let child = self.fetch_block(link.cid, work).await?;
                let leaf_data;
                let bytes: &[u8] = if link.cid.codec == 0x55 {
                    &child
                } else {
                    let leaf = car::decode_pbnode_lenient(&child).map_err(FetchError::permanent)?;
                    if leaf.unixfs.node_type != car::UNIXFS_FILE || !leaf.links.is_empty() {
                        return Err(FetchError::Permanent(eyre!(
                            "multi-level chunked file (outside the supported layout)"
                        )));
                    }
                    leaf_data = leaf.unixfs.data.unwrap_or_default();
                    &leaf_data
                };
                out.extend_from_slice(bytes);
                if out.len() > MAX_ITEM_BYTES {
                    return Err(FetchError::Permanent(eyre!(
                        "item file exceeds {MAX_ITEM_BYTES} bytes"
                    )));
                }
            }
            return Ok(out);
        }
    }

    /// The item file behind an item path, parsed into its `columns` and
    /// `values`; a parse failure of hash-verified bytes is permanent.
    pub async fn fetch_item_file(
        &self,
        path: &str,
        work: &mut ItemWork,
    ) -> Result<ItemFile, FetchError> {
        let (cid, segments) = parse_ipfs_path(path).map_err(FetchError::permanent)?;
        let bytes = self.fetch_item_bytes(cid, &segments, work).await?;
        ItemFile::parse(&bytes).map_err(FetchError::permanent)
    }
}

/// An item file: its `columns` (verbatim; null when absent) and `values`
/// (keyed by label). Deserialized straight into owned fields.
#[derive(Debug, Clone, Deserialize)]
pub struct ItemFile {
    #[serde(default)]
    pub columns: serde_json::Value,
    pub values: serde_json::Map<String, serde_json::Value>,
}

impl ItemFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).wrap_err("item JSON")
    }
}

// ---------- bounded scheduling ----------

/// One round of work on an item, as the scheduler sees it.
pub enum Round<T, S> {
    Done(T),
    Failed(eyre::Report),
    /// Try again no sooner than `after` from now, carrying `state`; until
    /// then the item waits in the delayed queue, not as an admitted task.
    Retry {
        after: Duration,
        state: S,
    },
}

/// Run `round` over every item with at most `concurrency` tasks admitted at a
/// time: an item is spawned only when a slot is free, a completion frees the
/// slot, and delayed retries wait in a queue of their own (one entry per item)
/// until due, then go first. Every task keeps its item id, so a task that
/// fails (a panic) records an error under that item.
pub async fn run_bounded<T, S, F, Fut>(
    items: Vec<(B256, String)>,
    concurrency: usize,
    mut round: F,
) -> BTreeMap<B256, Result<T>>
where
    T: Send + 'static,
    S: Send + 'static,
    F: FnMut(B256, Arc<str>, Option<S>) -> Fut,
    Fut: std::future::Future<Output = Round<T, S>> + Send + 'static,
{
    let concurrency = concurrency.max(1);
    let mut queue: VecDeque<(B256, Arc<str>, Option<S>)> = items
        .into_iter()
        .map(|(id, path)| (id, Arc::from(path), None))
        .collect();
    let mut delayed: BTreeMap<(Instant, u64), (B256, Arc<str>, S)> = BTreeMap::new();
    let mut seq = 0u64;
    let mut set: JoinSet<Round<T, S>> = JoinSet::new();
    let mut running: HashMap<tokio::task::Id, (B256, Arc<str>)> = HashMap::new();
    let mut out = BTreeMap::new();
    loop {
        let now = Instant::now();
        while delayed.first_key_value().is_some_and(|(k, _)| k.0 <= now) {
            if let Some((_, (id, path, state))) = delayed.pop_first() {
                queue.push_front((id, path, Some(state)));
            }
        }
        while set.len() < concurrency {
            let Some((id, path, state)) = queue.pop_front() else {
                break;
            };
            let handle = set.spawn(round(id, path.clone(), state));
            running.insert(handle.id(), (id, path));
        }
        if set.is_empty() {
            match delayed.first_key_value() {
                Some((&(due, _), _)) => {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
                    continue;
                }
                None => break,
            }
        }
        let next_due = delayed.first_key_value().map(|(&(due, _), _)| due);
        let joined = match next_due {
            Some(due) => tokio::select! {
                j = set.join_next_with_id() => j,
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(due)) => continue,
            },
            None => set.join_next_with_id().await,
        };
        let Some(joined) = joined else {
            continue;
        };
        match joined {
            Ok((tid, outcome)) => {
                let Some((id, path)) = running.remove(&tid) else {
                    continue;
                };
                match outcome {
                    Round::Done(v) => {
                        out.insert(id, Ok(v));
                    }
                    Round::Failed(e) => {
                        out.insert(id, Err(e));
                    }
                    Round::Retry { after, state } => {
                        seq += 1;
                        delayed.insert((Instant::now() + after, seq), (id, path, state));
                    }
                }
            }
            Err(e) => {
                if let Some((id, _)) = running.remove(&e.id()) {
                    out.insert(id, Err(eyre!("fetch task failed: {e}")));
                }
            }
        }
    }
    out
}

/// Fetch every included item's file through `fetcher`, at most `concurrency`
/// items admitted at a time (and at most `concurrency` block requests in
/// flight, the fetcher's own permit). A transient failure puts the item in
/// the delayed queue for another round within its budget; a permanent one is
/// recorded as is.
pub async fn fetch_all_files(
    fetcher: Arc<Fetcher>,
    paths: Vec<(B256, String)>,
    concurrency: usize,
) -> BTreeMap<B256, Result<ItemFile>> {
    run_bounded(
        paths,
        concurrency,
        move |_id, path, work: Option<ItemWork>| {
            let f = fetcher.clone();
            async move {
                let mut work = work.unwrap_or_else(|| ItemWork::new(f.budget()));
                work.rounds += 1;
                work.blocks = 0;
                match f.fetch_item_file(&path, &mut work).await {
                    Ok(v) => Round::Done(v),
                    Err(FetchError::Permanent(e)) => Round::Failed(e),
                    Err(FetchError::Transient(e)) => {
                        let backoff = f.budget().retry_backoff * work.rounds;
                        if work.rounds < f.budget().max_rounds
                            && Instant::now() + backoff < work.deadline
                        {
                            f.stats.item_retries.fetch_add(1, Ordering::Relaxed);
                            Round::Retry {
                                after: backoff,
                                state: work,
                            }
                        } else {
                            Round::Failed(e.wrap_err(format!("after {} rounds", work.rounds)))
                        }
                    }
                }
            }
        },
    )
    .await
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

/// The default list name. The Uniswap schema's `properties.name` is
/// `^[\w ]+$` (ASCII letters, digits, underscore, space), 1 to 30
/// characters: no comma, no hyphen.
pub const DEFAULT_LIST_NAME: &str = "Kleros Tokens Verified";

/// Limits the Uniswap token-list schema states (vendored, pinned, in
/// `fixtures/uniswap/`; `schema_checks_match_the_vendored_schema` holds
/// these to that file).
pub const LIST_NAME_MIN_CHARS: usize = 1;
pub const LIST_NAME_MAX_CHARS: usize = 30;
pub const TOKEN_NAME_MAX_CHARS: usize = 60;
pub const TOKEN_SYMBOL_MAX_CHARS: usize = 20;
pub const TOKENS_MIN: usize = 1;
pub const TOKENS_MAX: usize = 10_000;

/// The schema's list name: `^[\w ]+$`, 1 to 30 characters.
pub fn validate_list_name(name: &str) -> Result<()> {
    let n = name.chars().count();
    if !(LIST_NAME_MIN_CHARS..=LIST_NAME_MAX_CHARS).contains(&n) {
        bail!(
            "token-list name {name:?} must be {LIST_NAME_MIN_CHARS} to {LIST_NAME_MAX_CHARS} characters (schema properties.name)"
        );
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == ' '))
    {
        bail!(
            "token-list name {name:?} contains {c:?}; the schema allows ASCII letters, digits, underscore and space (^[\\w ]+$)"
        );
    }
    Ok(())
}

/// The schema's TokenInfo checks over one token: a positive chainId, an
/// EVM address (`0x` and 40 hex digits; this export emits no other form),
/// decimals 0 to 255 (held by the type), a name of 1 to 60 characters with
/// no whitespace other than spaces (`^[ \S+]+$`), a symbol of 1 to 20
/// characters with no whitespace (`^\S+$`). The schema also admits an empty
/// name or symbol; this export does not.
pub fn validate_token(t: &Token) -> Result<()> {
    if t.chain_id < 1 {
        bail!("chainId {} is not positive", t.chain_id);
    }
    let hex = t.address.strip_prefix("0x").unwrap_or_default();
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("address {:?} is not 0x and 40 hex digits", t.address);
    }
    let name_chars = t.name.chars().count();
    if name_chars == 0 || name_chars > TOKEN_NAME_MAX_CHARS {
        bail!(
            "name {:?} must be 1 to {TOKEN_NAME_MAX_CHARS} characters",
            t.name
        );
    }
    if let Some(c) = t.name.chars().find(|c| c.is_whitespace() && *c != ' ') {
        bail!(
            "name {:?} contains whitespace {c:?} other than a space",
            t.name
        );
    }
    let symbol_chars = t.symbol.chars().count();
    if symbol_chars == 0 || symbol_chars > TOKEN_SYMBOL_MAX_CHARS {
        bail!(
            "symbol {:?} must be 1 to {TOKEN_SYMBOL_MAX_CHARS} characters",
            t.symbol
        );
    }
    if let Some(c) = t.symbol.chars().find(|c| c.is_whitespace()) {
        bail!("symbol {:?} contains whitespace {c:?}", t.symbol);
    }
    Ok(())
}

/// `YYYY-MM-DDTHH:MM:SSZ`, the RFC 3339 UTC form `rfc3339` writes (the
/// schema's `date-time`).
pub fn validate_timestamp(ts: &str) -> Result<()> {
    let b = ts.as_bytes();
    let shape = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .all(|i| b[*i].is_ascii_digit());
    if !shape {
        bail!("timestamp {ts:?} is not YYYY-MM-DDTHH:MM:SSZ");
    }
    let num = |a: usize, z: usize| ts[a..z].parse::<u32>().unwrap_or(u32::MAX);
    let (month, day, hour, minute, second) =
        (num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19));
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        bail!("timestamp {ts:?} has a field out of range");
    }
    Ok(())
}

/// The rendered list against the schema's checks: the name, the timestamp,
/// non-negative version integers (held by the type), 1 to 10000 tokens,
/// every token valid. Run before the list is written; a failure is a bug in
/// the rendering, not a property of the data.
pub fn validate_token_list(list: &TokenList) -> Result<()> {
    validate_list_name(&list.name)?;
    validate_timestamp(&list.timestamp)?;
    if !(TOKENS_MIN..=TOKENS_MAX).contains(&list.tokens.len()) {
        bail!(
            "{} tokens; the schema allows {TOKENS_MIN} to {TOKENS_MAX}",
            list.tokens.len()
        );
    }
    for (i, t) in list.tokens.iter().enumerate() {
        validate_token(t).wrap_err_with(|| format!("token {i} ({}:{})", t.chain_id, t.address))?;
    }
    Ok(())
}

/// A rejected item and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub reason: String,
}

/// The skip reason for an Address value without the `eip155` namespace: the
/// Tokens list also holds Solana tokens under `solana:`, and a plain address
/// names no chain. No chain is guessed; Kleros's own exporter
/// (kleros/t2cr-to-ipfs) applied the same rule, so a comparison with its list
/// stays like-for-like.
pub const NO_EIP155_NAMESPACE: &str =
    "address has no eip155 namespace; skipped, as the discontinued Kleros export did";

/// Parse a CAIP-10 style "eip155:<chainId>:<address>" value. The address is
/// accepted in any case (list submitters do not always checksum) and
/// re-emitted EIP-55 checksummed.
pub fn parse_rich_address(raw: &str) -> Result<(u64, Address)> {
    let mut parts = raw.trim().split(':');
    let ns = parts.next().unwrap_or_default();
    if ns != "eip155" {
        bail!("{NO_EIP155_NAMESPACE}");
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
    let token = Token {
        chain_id,
        address: address.to_checksum(None),
        name,
        symbol,
        decimals: decimals as u8,
        logo_uri,
    };
    validate_token(&token)?;
    Ok(token)
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

/// Wall-clock seconds per stage of the export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct StageSeconds {
    pub identity: f64,
    pub anchor: f64,
    pub enumeration: f64,
    pub proofs: f64,
    pub content: f64,
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
    /// JSON-RPC requests issued (measured; `rpc` breaks them down).
    #[serde(rename = "rpcCalls")]
    pub rpc_calls: u64,
    /// Measured JSON-RPC work: requests, HTTP attempts, retries, bytes.
    #[serde(default)]
    pub rpc: RpcCounts,
    /// Measured content work: block requests, bytes, bad bytes, cache hits,
    /// coalesced requests, item retries.
    #[serde(default)]
    pub ipfs: IpfsCounts,
    #[serde(rename = "stageSeconds", default)]
    pub stage_seconds: StageSeconds,
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
    /// Anchor at this finalized height instead of the current one (a rerun
    /// at an earlier export's anchor; every anchor source must have
    /// finalized it).
    pub anchor_block: Option<u64>,
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
    /// JSON-RPC requests issued, measured at the transport.
    pub rpc_calls: u64,
    pub rpc: RpcCounts,
    pub ipfs: IpfsCounts,
    pub stages: StageSeconds,
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
    let stats = Arc::new(RpcStats::default());
    let mut stages = StageSeconds::default();
    let mut clock = Instant::now();
    let lap = |clock: &mut Instant| {
        let s = clock.elapsed().as_secs_f64();
        *clock = Instant::now();
        s
    };

    // 1. Chain identity of every distinct source, once; the authenticated
    //    sources (and their transports) serve every later step.
    let mut urls: Vec<&String> = cfg.anchor_rpcs.iter().chain(cfg.log_rpcs.iter()).collect();
    urls.push(&cfg.provider_rpc);
    urls.sort();
    urls.dedup();
    let mut sources: BTreeMap<String, Source> = BTreeMap::new();
    for rpc in &urls {
        let source = Source::authenticate(rpc, &cfg.pin, &stats).await?;
        sources.insert((*rpc).clone(), source);
    }
    stages.identity = lap(&mut clock);
    progress(format!(
        "chain identity: {} sources serve chain {} with the pinned genesis",
        sources.len(),
        cfg.pin.chain_id
    ));

    // 2. Anchor: each anchor source's finalized head, then the header every
    //    source serves at the agreed height (or at the pinned one).
    let anchor_sources: Vec<Source> = cfg
        .anchor_rpcs
        .iter()
        .map(|rpc| sources[rpc].clone())
        .collect();
    let (quorum, anchor_mode) = match cfg.anchor_block {
        None => (
            anchor::finalized_quorum_sources(&anchor_sources).await?,
            "header-quorum (alpha)".to_string(),
        ),
        Some(number) => (
            anchor::pinned_quorum_sources(&anchor_sources, number).await?,
            "header-quorum at a pinned finalized height (alpha)".to_string(),
        ),
    };
    if quorum.state_root == B256::ZERO {
        bail!("anchor header carries a zero state root; refusing");
    }
    stages.anchor = lap(&mut clock);
    progress(format!(
        "anchor: block {} {} ({} sources agree)",
        quorum.block_number, quorum.block_hash, quorum.sources
    ));

    // 3. Enumeration from every log source; the sets must agree.
    let log_sources: Vec<&Source> = cfg.log_rpcs.iter().map(|rpc| &sources[rpc]).collect();
    let (from_block, from_source) = match cfg.from_block {
        Some(b) => (b, "flag or preset".to_string()),
        None => {
            let (b, _) = discover_start(
                log_sources[0],
                cfg.list,
                quorum.block_number,
                cfg.log_window,
            )
            .await?;
            (
                b,
                format!(
                    "discovered via {} (first window with no logs and no code)",
                    log_sources[0].rpc
                ),
            )
        }
    };
    progress(format!(
        "enumerating NewItem logs from block {from_block} to {}",
        quorum.block_number
    ));
    let mut enums: Vec<(String, Enumeration)> = Vec::new();
    for source in &log_sources {
        let e = enumerate(
            source,
            cfg.list,
            from_block,
            quorum.block_number,
            cfg.log_window,
        )
        .await?;
        progress(format!(
            "  {}: {} logs, {} items, {} path/id mismatches, {} calls",
            source.rpc,
            e.logs,
            e.items.len(),
            e.mismatched.len(),
            e.rpc_calls
        ));
        enums.push((source.rpc.clone(), e));
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
    stages.enumeration = lap(&mut clock);

    // 4. Status proofs at the anchor.
    let proven = prove_statuses(
        &sources[&cfg.provider_rpc],
        cfg.list,
        cfg.items_slot,
        cfg.code_hash,
        &quorum,
        &ids,
    )
    .await?;
    let mut by_status: BTreeMap<String, u64> = BTreeMap::new();
    for s in proven.statuses.values() {
        *by_status.entry(status_name(*s).to_string()).or_default() += 1;
    }
    stages.proofs = lap(&mut clock);
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
    let fetcher = Arc::new(Fetcher::new(&cfg.gateways, cfg.concurrency));
    let mut fetched = fetch_all_files(fetcher.clone(), wanted, cfg.concurrency).await;
    let (items, fetch_failures) = records_from(
        &enumeration.items,
        &proven.statuses,
        &cfg.filter,
        &mut fetched,
    );
    stages.content = lap(&mut clock);
    let ipfs = fetcher.counts();
    progress(format!(
        "content: {} block requests, {} bytes, {} bad-byte responses, {} cache hits, {} item retries, {} failures",
        ipfs.requests, ipfs.response_bytes, ipfs.bad_bytes, ipfs.cache_hits, ipfs.item_retries, fetch_failures
    ));
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
    let rpc = stats.counts();
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
        rpc_calls: rpc.requests,
        rpc,
        ipfs,
        stages,
        anchor_mode,
    })
}

/// The snapshot's records from the enumerated paths, the proven statuses and
/// the fetch results, in item-id order; the fetched payloads move into the
/// records. Returns the records and the number of items whose fetch failed
/// (their record carries the error).
pub fn records_from(
    paths: &BTreeMap<B256, String>,
    statuses: &BTreeMap<B256, u8>,
    filter: &StatusFilter,
    fetched: &mut BTreeMap<B256, Result<ItemFile>>,
) -> (Vec<ItemRecord>, u64) {
    let mut items = Vec::new();
    let mut fetch_failures = 0u64;
    for (id, path) in paths {
        let status = statuses[id];
        if !filter.includes(status) {
            continue;
        }
        let (columns, values, error) = match fetched.remove(id) {
            Some(Ok(f)) => (f.columns, serde_json::Value::Object(f.values), None),
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
    (items, fetch_failures)
}

/// Tokens from a snapshot: every Registered item's values (plus
/// ClearingRequested with `include_clearing`) through `token_from_values`.
pub fn tokens_from_snapshot(
    snapshot: &ListSnapshot,
    include_clearing: bool,
    logo_base: &str,
) -> Result<(Vec<Token>, Vec<Skipped>)> {
    let mut inputs: ItemInputs<'_> = BTreeMap::new();
    for it in &snapshot.items {
        let id: B256 = it
            .item_id
            .parse()
            .map_err(|e| eyre!("item id {}: {e}", it.item_id))?;
        inputs.insert(id, (it.status, it.values.as_object()));
    }
    Ok(assemble_tokens(&inputs, include_clearing, logo_base))
}

/// itemID → (status, the item's values when fetched), borrowed from the
/// snapshot.
pub type ItemInputs<'a> =
    BTreeMap<B256, (u8, Option<&'a serde_json::Map<String, serde_json::Value>>)>;

/// The part of the export that is pure computation, kept apart for tests:
/// given proven statuses and fetched values, produce the token set in its
/// canonical (chainId, lowercase address) order and the skip list. Duplicate
/// (chainId, address) pairs keep the item with the lowest item id.
pub fn assemble_tokens(
    items: &ItemInputs<'_>,
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
    // The map's key is the canonical order (`sort_tokens`); no second sort.
    let out: Vec<Token> = tokens.into_values().map(|(_, t)| t).collect();
    skipped.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    (out, skipped)
}

/// sha256 of the exact bytes a step of the pipeline produced — used for the
/// provenance's output digests (matching `sha256sum`).
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
        assert_eq!(
            parse_rich_address("cosmos:1:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
                .unwrap_err()
                .to_string(),
            NO_EIP155_NAMESPACE
        );
        assert_eq!(
            parse_rich_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
                .unwrap_err()
                .to_string(),
            NO_EIP155_NAMESPACE,
            "a plain address gets no guessed chain"
        );
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
        // TokenInfo constraints: skipped with the reason, never emitted.
        let mut bad = v.clone();
        bad.insert("Symbol".into(), serde_json::json!("US DC"));
        assert!(token_from_values(&bad, "ipfs://")
            .unwrap_err()
            .to_string()
            .contains("whitespace"));
        let mut bad = v.clone();
        bad.insert("Name".into(), serde_json::json!("a\tb"));
        assert!(token_from_values(&bad, "ipfs://")
            .unwrap_err()
            .to_string()
            .contains("whitespace"));
        let mut bad = v.clone();
        bad.insert("Name".into(), serde_json::json!("n".repeat(61)));
        assert!(token_from_values(&bad, "ipfs://")
            .unwrap_err()
            .to_string()
            .contains("1 to 60"));
        let mut bad = v.clone();
        bad.insert("Symbol".into(), serde_json::json!("S".repeat(21)));
        assert!(token_from_values(&bad, "ipfs://").is_err());
        let mut bad = v.clone();
        bad.insert(
            "Address".into(),
            serde_json::json!("eip155:0:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
        );
        assert!(token_from_values(&bad, "ipfs://")
            .unwrap_err()
            .to_string()
            .contains("not positive"));
    }

    #[test]
    fn default_rendering_passes_the_schema_checks() {
        validate_list_name(DEFAULT_LIST_NAME).unwrap();
        let old = validate_list_name("Kleros Tokens, verified export").unwrap_err();
        assert!(old.to_string().contains("','"), "{old}");
        assert!(validate_list_name("").is_err());
        assert!(validate_list_name(&"n".repeat(31)).is_err());
        assert!(validate_list_name("Kleros-Tokens").is_err());
        assert!(validate_list_name("Kleros_Tokens 2").is_ok());

        let v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"Address":"eip155:1:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","Name":"USD Coin","Symbol":"USDC","Decimals":"6","Logo":"/ipfs/QmLogo/usdc.png"}"#,
        )
        .unwrap();
        let t = token_from_values(&v, "ipfs://").unwrap();
        let list = TokenList {
            name: DEFAULT_LIST_NAME.into(),
            timestamp: rfc3339(1_788_817_292),
            version: bump_version(None, std::slice::from_ref(&t)),
            tokens: vec![t],
        };
        validate_token_list(&list).unwrap();
        // The bytes written pass too.
        let back: TokenList = serde_json::from_slice(&serialize_list(&list).unwrap()).unwrap();
        validate_token_list(&back).unwrap();
        let mut bad = list.clone();
        bad.name = "Kleros Tokens, verified export".into();
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.tokens.clear();
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.timestamp = "2026-09-07 21:41:32".into();
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.timestamp = "2026-13-07T21:41:32Z".into();
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.tokens[0].chain_id = 0;
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.tokens[0].symbol = "US DC".into();
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list.clone();
        bad.tokens[0].name = "a\u{a0}b".into();
        assert!(
            validate_token_list(&bad).is_err(),
            "NBSP is whitespace to \\S"
        );
        let mut bad = list.clone();
        bad.tokens[0].name = "x".repeat(61);
        assert!(validate_token_list(&bad).is_err());
        let mut bad = list;
        bad.tokens[0].address = "0x1234".into();
        assert!(validate_token_list(&bad).is_err());
    }

    /// The vendored schema (fixtures/uniswap/tokenlist.schema.json, MIT,
    /// pinned in PROVENANCE.txt) is what the constants above encode; a drift
    /// fails here.
    #[test]
    fn schema_checks_match_the_vendored_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/uniswap/tokenlist.schema.json"))
                .unwrap();
        let name = &schema["properties"]["name"];
        assert_eq!(name["pattern"], "^[\\w ]+$");
        assert_eq!(name["minLength"], LIST_NAME_MIN_CHARS);
        assert_eq!(name["maxLength"], LIST_NAME_MAX_CHARS);
        assert_eq!(schema["properties"]["timestamp"]["format"], "date-time");
        let tokens = &schema["properties"]["tokens"];
        assert_eq!(tokens["minItems"], TOKENS_MIN);
        assert_eq!(tokens["maxItems"], TOKENS_MAX);
        assert_eq!(
            schema["required"],
            serde_json::json!(["name", "timestamp", "version", "tokens"])
        );
        let version = &schema["definitions"]["Version"]["properties"];
        for field in ["major", "minor", "patch"] {
            assert_eq!(version[field]["type"], "integer");
            assert_eq!(version[field]["minimum"], 0);
        }
        let info = &schema["definitions"]["TokenInfo"]["properties"];
        assert_eq!(info["chainId"]["minimum"], 1);
        assert!(info["address"]["pattern"]
            .as_str()
            .unwrap()
            .contains("0x[a-fA-F0-9]{40}"));
        assert_eq!(info["decimals"]["minimum"], 0);
        assert_eq!(info["decimals"]["maximum"], 255);
        assert_eq!(info["name"]["maxLength"], TOKEN_NAME_MAX_CHARS);
        assert_eq!(info["name"]["anyOf"][1]["pattern"], "^[ \\S+]+$");
        assert_eq!(info["symbol"]["maxLength"], TOKEN_SYMBOL_MAX_CHARS);
        assert_eq!(info["symbol"]["anyOf"][1]["pattern"], "^\\S+$");
        assert_eq!(
            schema["definitions"]["TokenInfo"]["required"],
            serde_json::json!(["chainId", "address", "decimals", "name", "symbol"])
        );
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
        let id6 = b256!("0000000000000000000000000000000000000000000000000000000000000006");
        let v2 = mk("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "18");
        let v1 = mk("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "18"); // duplicate, lower id wins
        let v3 = mk("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "6");
        let v5 = mk("0xcccccccccccccccccccccccccccccccccccccccc", "999");
        let v6 = mk("0x9999999999999999999999999999999999999999", "0");
        let mut items: ItemInputs<'_> = BTreeMap::new();
        items.insert(id2, (STATUS_REGISTERED, Some(&v2)));
        items.insert(id1, (STATUS_REGISTERED, Some(&v1)));
        items.insert(id3, (STATUS_CLEARING_REQUESTED, Some(&v3)));
        items.insert(id4, (STATUS_REGISTERED, None));
        items.insert(id5, (STATUS_REGISTERED, Some(&v5)));
        items.insert(id6, (STATUS_REGISTERED, Some(&v6)));
        let (tokens, skipped) = assemble_tokens(&items, false, "ipfs://");
        assert_eq!(tokens.len(), 2);
        // Canonical order straight out of the map: (chainId, lowercase address).
        let mut sorted = tokens.clone();
        sort_tokens(&mut sorted);
        assert_eq!(tokens, sorted);
        assert_eq!(
            tokens[0].address.to_lowercase(),
            "0x9999999999999999999999999999999999999999"
        );
        assert_eq!(
            tokens[1].address.to_lowercase(),
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
        assert_eq!(tokens.len(), 3);
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
        assert_eq!(skipped[0].reason, NO_EIP155_NAMESPACE);
        let (tokens, _) = tokens_from_snapshot(&snap, true, "ipfs://").unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn log_errors_are_classified_by_recognised_formats_only() {
        use LogError::*;
        let named = |n| RangeLimit { named: Some(n) };
        assert_eq!(classify_log_error("server returned an error response: error code -32701: exceed maximum block range: 50000"), named(50_000));
        assert_eq!(
            classify_log_error(
                "eth_getLogs and eth_newFilter are limited to a 10,000 blocks range"
            ),
            named(10_000)
        );
        assert_eq!(classify_log_error("Log response size exceeded. You can make eth_getLogs requests with up to a 2K block range and no limit on the response size, or you can request any block range with a cap of 10K logs in the response. Based on your parameters and the response size limit, this block range should work: [0x2dc6c0, 0x2dc6c5]"), named(6));
        assert_eq!(
            classify_log_error(
                "query returned more than 10000 results. Try with this block range [0x100, 0x1FF]."
            ),
            named(256)
        );
        assert_eq!(
            classify_log_error("Block range is too large: max 1000 blocks"),
            named(1000)
        );
        assert_eq!(
            classify_log_error("block range of 5000 exceeded"),
            named(5000)
        );
        // A number that is not the limit is never taken for it.
        assert_eq!(
            classify_log_error("block range too large (request id 4200)"),
            RangeLimit { named: None }
        );
        assert_eq!(
            classify_log_error("request 48170840 failed: block range too large"),
            RangeLimit { named: None }
        );
        assert_eq!(
            classify_log_error("query returned more than 10000 results"),
            RangeLimit { named: None }
        );
        assert_eq!(
            classify_log_error("block range is too wide"),
            RangeLimit { named: None }
        );
        // Not limits at all.
        assert_eq!(
            classify_log_error("https://rpc.example eth_getLogs [1, 2]: deadline of 30s exceeded"),
            Timeout
        );
        assert_eq!(
            classify_log_error("HTTP 504 Gateway Timeout from https://rpc.example"),
            Timeout
        );
        assert_eq!(
            classify_log_error("error code -32005: rate limit exceeded, retry in 4200 ms"),
            Transient
        );
        assert_eq!(
            classify_log_error("HTTP 503 Service Unavailable"),
            Transient
        );
        assert_eq!(classify_log_error("connection reset by peer"), Transient);
    }

    #[test]
    fn next_window_after_a_limit() {
        let named = |n| LogError::RangeLimit { named: Some(n) };
        let unnamed = LogError::RangeLimit { named: None };
        assert_eq!(next_window(1_000_000, &named(50_000), 1000), Some(50_000));
        assert_eq!(
            next_window(40_000, &named(50_000), 1000),
            Some(20_000),
            "a limit above the window halves"
        );
        assert_eq!(next_window(1_000_000, &unnamed, 1000), Some(500_000));
        assert_eq!(next_window(1500, &unnamed, 1000), Some(1000));
        assert_eq!(
            next_window(1000, &unnamed, 1000),
            None,
            "already at the smallest window"
        );
        assert_eq!(
            next_window(1_000_000, &LogError::Transient, 1000),
            Some(1_000_000),
            "not a limit: same window"
        );
        assert_eq!(
            next_window(1_000_000, &LogError::Timeout, 1000),
            Some(1_000_000)
        );
    }

    // ---------- enumeration: a JSON-RPC source on localhost ----------

    const TEST_GENESIS: B256 =
        b256!("4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756");

    /// eth_getLogs behaviour: given the call index and the window, logs or a
    /// JSON-RPC error (code, message).
    type LogsScript =
        dyn Fn(usize, u64, u64) -> Result<Vec<serde_json::Value>, (i64, String)> + Send + Sync;

    fn hex_u64(v: &serde_json::Value) -> u64 {
        u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    /// The eth_getLogs windows a mock source was asked for, in order.
    type Windows = Arc<std::sync::Mutex<Vec<(u64, u64)>>>;

    /// A JSON-RPC server answering eth_chainId (100), the genesis hash,
    /// eth_getCode (empty) and eth_getLogs per `script`, logging every
    /// getLogs window in order.
    fn mock_rpc(script: Arc<LogsScript>) -> (String, Windows) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let windows: Windows = Arc::new(std::sync::Mutex::new(Vec::new()));
        let windows2 = windows.clone();
        std::thread::spawn(move || {
            let calls = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                let body_start;
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        body_start = p + 4;
                        let head = String::from_utf8_lossy(&raw[..p]).to_string();
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        while raw.len() < body_start + len {
                            let n = stream.read(&mut buf).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            raw.extend_from_slice(&buf[..n]);
                        }
                        break;
                    }
                }
                let Some(body_start) = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
                else {
                    continue;
                };
                let req: serde_json::Value = serde_json::from_slice(&raw[body_start..]).unwrap();
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap_or_default();
                let params = &req["params"];
                let reply = match method {
                    "eth_chainId" => Ok(serde_json::json!("0x64")),
                    "eth_getBlockByNumber" => {
                        Ok(serde_json::json!({"hash": TEST_GENESIS.to_string(), "number": "0x0"}))
                    }
                    "eth_getCode" => Ok(serde_json::json!("0x")),
                    "eth_getLogs" => {
                        let from = hex_u64(&params[0]["fromBlock"]);
                        let to = hex_u64(&params[0]["toBlock"]);
                        windows2.lock().unwrap().push((from, to));
                        let i = calls.fetch_add(1, AtomicOrdering::SeqCst);
                        script(i, from, to).map(serde_json::Value::Array)
                    }
                    other => Err((-32601, format!("unknown method {other}"))),
                };
                let body = match reply {
                    Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err((code, message)) => serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
                }
                .to_string();
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.write_all(body.as_bytes());
            }
        });
        (format!("http://{addr}/"), windows)
    }

    async fn test_source(url: &str) -> Source {
        let pin = ChainPin {
            chain_id: 100,
            genesis_hash: TEST_GENESIS,
        };
        Source::authenticate(url, &pin, &Arc::default())
            .await
            .unwrap()
    }

    fn fast_policy(window: u64) -> LogWindowPolicy {
        LogWindowPolicy {
            backoff: Duration::from_millis(5),
            ..LogWindowPolicy::new(window)
        }
    }

    fn new_item_log(list: Address, id: B256, path: &str) -> serde_json::Value {
        use alloy::sol_types::SolValue;
        let data = (path.to_string(), false).abi_encode_params();
        serde_json::json!({
            "address": list.to_string(),
            "topics": [NewItem::SIGNATURE_HASH.to_string(), id.to_string()],
            "data": format!("0x{}", car::hex_lower(&data)),
            "blockNumber": "0x10",
            "transactionHash": B256::repeat_byte(0xab).to_string(),
            "transactionIndex": "0x0",
            "blockHash": B256::repeat_byte(0xcd).to_string(),
            "logIndex": "0x0",
            "removed": false,
        })
    }

    #[tokio::test]
    async fn log_scan_retries_a_transient_failure_at_the_same_window() {
        let script: Arc<LogsScript> = Arc::new(|i, _from, _to| {
            if i == 3 {
                Err((-32603, "internal error: too many requests".to_string()))
            } else {
                Ok(Vec::new())
            }
        });
        let (url, windows) = mock_rpc(script);
        let source = test_source(&url).await;
        let list = Address::repeat_byte(0x11);
        let e = enumerate_with(&source, list, 0, 9_999, &fast_policy(1000))
            .await
            .unwrap();
        let w = windows.lock().unwrap();
        assert_eq!(w.len(), 11, "ten windows plus one retry: {w:?}");
        assert!(
            w.iter().all(|(f, t)| t - f + 1 == 1000),
            "a transient failure never shrinks the window: {w:?}"
        );
        assert_eq!(w[3], w[4], "the failed window is retried as is");
        assert_eq!((e.rpc_calls, e.retries, e.shrinks, e.grows), (11, 1, 0, 0));
    }

    #[tokio::test]
    async fn log_scan_shrinks_to_a_named_limit_and_stays_there() {
        let script: Arc<LogsScript> = Arc::new(|_i, from, to| {
            if to - from + 1 > 2000 {
                Err((-32701, "exceed maximum block range: 2000".to_string()))
            } else {
                Ok(Vec::new())
            }
        });
        let (url, windows) = mock_rpc(script);
        let source = test_source(&url).await;
        let e = enumerate_with(
            &source,
            Address::repeat_byte(0x11),
            0,
            39_999,
            &fast_policy(8000),
        )
        .await
        .unwrap();
        let w = windows.lock().unwrap();
        assert_eq!(
            w.len(),
            21,
            "one refused window, then twenty of 2000: {w:?}"
        );
        assert!(w[1..].iter().all(|(f, t)| t - f + 1 == 2000));
        assert_eq!((e.rpc_calls, e.retries, e.shrinks, e.grows), (21, 0, 1, 0));
    }

    #[tokio::test]
    async fn log_scan_halves_on_an_unnamed_limit_and_grows_back_after_success() {
        // Windows over 2000 blocks are refused without naming the limit: the
        // scan halves 8000 → 4000 → 2000, succeeds, and after eight windows
        // tries 4000 again; that fails and it settles back, all bounded.
        let script: Arc<LogsScript> = Arc::new(|_i, from, to| {
            if to - from + 1 > 2000 {
                Err((
                    -32000,
                    "block range too large (request id 4200)".to_string(),
                ))
            } else {
                Ok(Vec::new())
            }
        });
        let (url, windows) = mock_rpc(script);
        let source = test_source(&url).await;
        let e = enumerate_with(
            &source,
            Address::repeat_byte(0x11),
            0,
            39_999,
            &fast_policy(8000),
        )
        .await
        .unwrap();
        let w = windows.lock().unwrap();
        assert_eq!(
            (w[0], w[1], w[2]),
            ((0, 7999), (0, 3999), (0, 1999)),
            "{w:?}"
        );
        assert!(e.grows >= 1, "sustained success grows the window: {e:?}");
        assert!(e.shrinks >= 3);
        assert!(
            w.len() <= 26,
            "growth attempts stay a small overhead: {} calls",
            w.len()
        );
        assert_eq!(w.last().unwrap().1, 39_999);
    }

    #[tokio::test]
    async fn log_scan_gives_up_after_bounded_retries_of_a_persistent_failure() {
        let script: Arc<LogsScript> =
            Arc::new(|_i, _f, _t| Err((-32005, "rate limit exceeded".to_string())));
        let (url, windows) = mock_rpc(script);
        let source = test_source(&url).await;
        let err = enumerate_with(
            &source,
            Address::repeat_byte(0x11),
            0,
            999,
            &fast_policy(1000),
        )
        .await
        .unwrap_err();
        assert_eq!(
            windows.lock().unwrap().len(),
            4,
            "one call and three retries"
        );
        assert!(format!("{err:#}").contains("failed 4 times"), "{err:#}");
    }

    #[tokio::test]
    async fn log_scan_decodes_items_and_drops_path_id_mismatches() {
        let list = Address::repeat_byte(0x11);
        let good_path = "/ipfs/QmWtvA69pfnBbkJvLS3TAJuevnKdb35NbrvTDuARQszAAv/item.json";
        let good_id = keccak256(good_path.as_bytes());
        let bad_id = B256::repeat_byte(0x99);
        let logs = vec![
            new_item_log(list, good_id, good_path),
            new_item_log(list, bad_id, good_path),
        ];
        let script: Arc<LogsScript> = Arc::new(move |_i, _f, _t| Ok(logs.clone()));
        let (url, _) = mock_rpc(script);
        let source = test_source(&url).await;
        let e = enumerate_with(&source, list, 0, 999, &fast_policy(1000))
            .await
            .unwrap();
        assert_eq!(e.logs, 2);
        assert_eq!(e.items.get(&good_id).map(String::as_str), Some(good_path));
        assert_eq!(e.mismatched, vec![(bad_id, good_path.to_string())]);
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

    // ---------- content pipeline: a gateway on localhost ----------

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// How the mock gateway answers one CID.
    enum Behaviour {
        Bytes(Vec<u8>),
        /// A transient status for the first `n` requests, then the bytes.
        FailFirst(usize, Vec<u8>),
        /// Answer after a delay (a stalled gateway).
        Stall(Duration, Vec<u8>),
    }

    /// A gateway serving `/ipfs/<cid>?format=raw` from a table, one thread per
    /// connection, logging every CID requested in order.
    fn mock_gateway(
        table: HashMap<String, Behaviour>,
    ) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let table = Arc::new(std::sync::Mutex::new(table));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let log = log2.clone();
                let table = table.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let line = req.lines().next().unwrap_or_default();
                    let path = line.split_whitespace().nth(1).unwrap_or_default();
                    let cid = path
                        .trim_start_matches("/ipfs/")
                        .split('?')
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    log.lock().unwrap().push(cid.clone());
                    let (code, body, delay) = {
                        let mut t = table.lock().unwrap();
                        match t.get_mut(&cid) {
                            None => (404, Vec::new(), None),
                            Some(Behaviour::Bytes(b)) => (200, b.clone(), None),
                            Some(Behaviour::FailFirst(n, b)) => {
                                if *n > 0 {
                                    *n -= 1;
                                    (503, Vec::new(), None)
                                } else {
                                    (200, b.clone(), None)
                                }
                            }
                            Some(Behaviour::Stall(d, b)) => (200, b.clone(), Some(*d)),
                        }
                    };
                    if let Some(d) = delay {
                        std::thread::sleep(d);
                    }
                    let header = format!(
                        "HTTP/1.1 {code} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                });
            }
        });
        (format!("http://{addr}"), log)
    }

    fn raw_block(data: &[u8]) -> (Cid, String) {
        let c = Cid::for_block(0x55, data);
        (c, c.to_string_canonical())
    }

    fn pb_block(bytes: &[u8]) -> (Cid, String) {
        let c = Cid::for_block(0x70, bytes);
        (c, c.to_string_v0().unwrap())
    }

    fn fast_budget() -> ItemBudget {
        ItemBudget {
            retry_backoff: Duration::from_millis(1),
            block_timeout: Duration::from_secs(5),
            ..Default::default()
        }
    }

    fn item_id(i: u64) -> B256 {
        B256::from(U256::from(i).to_be_bytes::<32>())
    }

    fn requests_for(log: &std::sync::Mutex<Vec<String>>, cid: &str) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|c| c.as_str() == cid)
            .count()
    }

    #[tokio::test]
    async fn repeated_links_to_one_block_are_fetched_once_and_budgeted() {
        let (empty, empty_text) = raw_block(b"");
        let small = car::encode_chunked_file(&vec![(empty, 0); 100]);
        let (small_cid, small_text) = pb_block(&small);
        let big = car::encode_chunked_file(&vec![(empty, 0); 1000]);
        let (big_cid, big_text) = pb_block(&big);
        let mut table = HashMap::new();
        table.insert(empty_text.clone(), Behaviour::Bytes(Vec::new()));
        table.insert(small_text, Behaviour::Bytes(small));
        table.insert(big_text.clone(), Behaviour::Bytes(big));
        let (url, log) = mock_gateway(table);
        let fetcher = Fetcher::new(&[url], 4).with_budget(fast_budget());

        // 100 links to one empty block: the block is fetched once, the file is empty.
        let mut work = ItemWork::new(fetcher.budget());
        let bytes = fetcher
            .fetch_item_bytes(small_cid, &[], &mut work)
            .await
            .unwrap();
        assert!(bytes.is_empty());
        assert_eq!(work.blocks, 101, "every link is a visit, cache hit or not");
        assert_eq!(work.attempts, 2);
        assert_eq!(
            log.lock().unwrap().len(),
            2,
            "the root and the empty block, once each"
        );
        assert_eq!(fetcher.counts().cache_hits, 99);

        // 1000 links: the block budget ends the walk; the network saw only the new root.
        let mut work = ItemWork::new(fetcher.budget());
        let err = fetcher
            .fetch_item_bytes(big_cid, &[], &mut work)
            .await
            .unwrap_err();
        assert!(!err.is_transient());
        assert!(
            err.to_string()
                .contains(&format!("more than {ITEM_MAX_BLOCKS} blocks")),
            "{err}"
        );
        assert_eq!(work.blocks, ITEM_MAX_BLOCKS + 1);
        assert_eq!(log.lock().unwrap().len(), 3);
        assert_eq!(requests_for(&log, &empty_text), 1);
    }

    #[tokio::test]
    async fn hash_verified_invalid_json_is_not_refetched() {
        let body = b"not json at all".to_vec();
        let (_, text) = raw_block(&body);
        let (url, log) = mock_gateway(HashMap::from([(text.clone(), Behaviour::Bytes(body))]));
        let fetcher = Arc::new(Fetcher::new(&[url], 2).with_budget(fast_budget()));
        let id = item_id(1);
        let out = fetch_all_files(fetcher.clone(), vec![(id, format!("/ipfs/{text}"))], 2).await;
        let err = format!("{:#}", out[&id].as_ref().unwrap_err());
        assert!(err.contains("item JSON"), "{err}");
        assert_eq!(
            requests_for(&log, &text),
            1,
            "verified bytes that do not parse are permanent: downloaded once"
        );
        assert_eq!(fetcher.counts().item_retries, 0);
        assert_eq!(fetcher.counts().requests, 1);
    }

    #[tokio::test]
    async fn transient_failure_on_a_later_chunk_reuses_earlier_blocks() {
        let json = br#"{"columns":[],"values":{"Name":"chunked"}}"#;
        let (c1_data, c2_data) = json.split_at(12);
        let (c1, t1) = raw_block(c1_data);
        let (c2, t2) = raw_block(c2_data);
        let root =
            car::encode_chunked_file(&[(c1, c1_data.len() as u64), (c2, c2_data.len() as u64)]);
        let (_, root_text) = pb_block(&root);
        let mut table = HashMap::new();
        table.insert(root_text.clone(), Behaviour::Bytes(root));
        table.insert(t1.clone(), Behaviour::Bytes(c1_data.to_vec()));
        table.insert(t2.clone(), Behaviour::FailFirst(1, c2_data.to_vec()));
        let (url, log) = mock_gateway(table);
        let fetcher = Arc::new(Fetcher::new(&[url], 2).with_budget(fast_budget()));
        let id = item_id(2);
        let out =
            fetch_all_files(fetcher.clone(), vec![(id, format!("/ipfs/{root_text}"))], 2).await;
        let file = out[&id].as_ref().unwrap();
        assert_eq!(file.values["Name"], "chunked");
        assert_eq!(
            requests_for(&log, &root_text),
            1,
            "the root was verified once"
        );
        assert_eq!(
            requests_for(&log, &t1),
            1,
            "the first chunk was verified once"
        );
        assert_eq!(
            requests_for(&log, &t2),
            2,
            "only the unavailable chunk was asked again"
        );
        let c = fetcher.counts();
        assert_eq!(c.item_retries, 1);
        assert_eq!(
            c.cache_hits, 2,
            "the retry round replayed the root and chunk 1 from the cache"
        );
        assert_eq!(c.requests, 4);
    }

    #[tokio::test]
    async fn wrong_bytes_from_one_gateway_fail_over_and_demote_it() {
        let correct = br#"{"values":{}}"#.to_vec();
        let (cid, text) = raw_block(&correct);
        let (bad_url, bad_log) = mock_gateway(HashMap::from([(
            text.clone(),
            Behaviour::Bytes(b"tampered".to_vec()),
        )]));
        let (good_url, _) =
            mock_gateway(HashMap::from([(text, Behaviour::Bytes(correct.clone()))]));
        let fetcher = Fetcher::new(&[bad_url, good_url], 2).with_budget(fast_budget());
        let mut work = ItemWork::new(fetcher.budget());
        let bytes = fetcher.fetch_item_bytes(cid, &[], &mut work).await.unwrap();
        assert_eq!(bytes, correct);
        assert_eq!(fetcher.counts().bad_bytes, 1);
        assert_eq!(work.attempts, 2);
        assert_eq!(bad_log.lock().unwrap().len(), 1);
        assert_eq!(
            fetcher.gateways().ordered(),
            vec![1, 0],
            "the gateway that served wrong bytes is tried last from now on"
        );
    }

    #[tokio::test]
    async fn concurrent_misses_for_one_block_coalesce_into_one_fetch() {
        let json = br#"{"values":{"k":"v"}}"#.to_vec();
        let (_, text) = raw_block(&json);
        let (url, log) = mock_gateway(HashMap::from([(
            text.clone(),
            Behaviour::Stall(Duration::from_millis(100), json),
        )]));
        let fetcher = Arc::new(Fetcher::new(&[url], 4).with_budget(fast_budget()));
        let path = format!("/ipfs/{text}");
        let out = fetch_all_files(
            fetcher.clone(),
            vec![(item_id(1), path.clone()), (item_id(2), path)],
            4,
        )
        .await;
        assert!(out.values().all(|r| r.is_ok()));
        assert_eq!(log.lock().unwrap().len(), 1, "one fetch served both items");
        let c = fetcher.counts();
        assert_eq!((c.requests, c.coalesced, c.cache_hits), (1, 1, 0));
    }

    #[tokio::test]
    async fn time_budget_bounds_a_stalled_gateway_and_shortens_the_request_deadline() {
        let body = b"x".to_vec();
        let (_, text) = raw_block(&body);
        let (url, _) = mock_gateway(HashMap::from([(
            text.clone(),
            Behaviour::Stall(Duration::from_secs(20), body),
        )]));
        let budget = ItemBudget {
            time: Duration::from_millis(300),
            block_timeout: Duration::from_secs(30),
            retry_backoff: Duration::from_millis(1),
            ..Default::default()
        };
        let fetcher = Arc::new(Fetcher::new(&[url], 1).with_budget(budget));
        let id = item_id(3);
        let started = Instant::now();
        let out = fetch_all_files(fetcher.clone(), vec![(id, format!("/ipfs/{text}"))], 1).await;
        let err = format!("{:#}", out[&id].as_ref().unwrap_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the request deadline must be what is left of the item's budget, not the 30 s block timeout ({err})"
        );
        assert!(
            err.contains("budget") || err.contains("timed out") || err.contains("no gateway"),
            "{err}"
        );
    }

    // ---------- bounded scheduling ----------

    #[tokio::test]
    async fn run_bounded_admits_at_most_concurrency_tasks() {
        let items: Vec<(B256, String)> = (1..=1000u64)
            .map(|i| (item_id(i), format!("/ipfs/{i}")))
            .collect();
        let admitted = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let out = run_bounded(items, 8, |_id, _path, _state: Option<()>| {
            // Counted when the task is admitted (spawned), released when it ends.
            let now = admitted.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            peak.fetch_max(now, AtomicOrdering::SeqCst);
            let admitted = admitted.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(1)).await;
                admitted.fetch_sub(1, AtomicOrdering::SeqCst);
                Round::Done(())
            }
        })
        .await;
        assert_eq!(out.len(), 1000);
        assert!(out.values().all(|r| r.is_ok()));
        assert_eq!(
            peak.load(AtomicOrdering::SeqCst),
            8,
            "admitted tasks, not just permit holders, stay at the limit"
        );
        assert_eq!(admitted.load(AtomicOrdering::SeqCst), 0);
    }

    #[tokio::test]
    async fn run_bounded_records_a_task_failure_under_its_item_id() {
        let ids = [item_id(1), item_id(2), item_id(3)];
        let items: Vec<(B256, String)> = ids.iter().map(|i| (*i, format!("/ipfs/{i}"))).collect();
        let out = run_bounded(items, 2, |id, _path, _state: Option<()>| async move {
            if id == ids[1] {
                panic!("injected task failure");
            }
            Round::Done(id)
        })
        .await;
        assert_eq!(out.len(), 3);
        assert!(!out.contains_key(&B256::ZERO), "no sentinel entry");
        assert_eq!(*out[&ids[0]].as_ref().unwrap(), ids[0]);
        assert_eq!(*out[&ids[2]].as_ref().unwrap(), ids[2]);
        let err = format!("{:#}", out[&ids[1]].as_ref().unwrap_err());
        assert!(err.contains("fetch task failed"), "{err}");

        // The snapshot records the failure on that item and counts it.
        let paths: BTreeMap<B256, String> =
            ids.iter().map(|i| (*i, format!("/ipfs/{i}"))).collect();
        let statuses: BTreeMap<B256, u8> = ids.iter().map(|i| (*i, STATUS_REGISTERED)).collect();
        let mut fetched: BTreeMap<B256, Result<ItemFile>> = out
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.map(|_| ItemFile {
                        columns: serde_json::Value::Null,
                        values: serde_json::Map::new(),
                    }),
                )
            })
            .collect();
        let filter = StatusFilter {
            pending: false,
            all: false,
        };
        let (records, failures) = records_from(&paths, &statuses, &filter, &mut fetched);
        assert_eq!(failures, 1);
        assert_eq!(records.len(), 3);
        let failed = records
            .iter()
            .find(|r| r.item_id == ids[1].to_string())
            .unwrap();
        assert!(failed
            .error
            .as_deref()
            .unwrap()
            .contains("fetch task failed"));
        assert!(failed.values.is_null());
        assert_eq!(records.iter().filter(|r| r.error.is_none()).count(), 2);
        assert!(fetched.is_empty(), "payloads moved into the records");
    }

    #[tokio::test]
    async fn run_bounded_carries_retry_state_through_the_delayed_queue() {
        let id = item_id(7);
        let started = Instant::now();
        let out = run_bounded(
            vec![(id, "/ipfs/x".into())],
            1,
            |_id, _path, state: Option<u32>| async move {
                match state {
                    None => Round::Retry {
                        after: Duration::from_millis(20),
                        state: 1,
                    },
                    Some(n) => Round::Done(n + 1),
                }
            },
        )
        .await;
        assert_eq!(*out[&id].as_ref().unwrap(), 2);
        assert!(started.elapsed() >= Duration::from_millis(20));
    }
}
