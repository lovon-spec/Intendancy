//! `curate-export` — verified exports of Kleros Light Curate lists: a
//! registry-agnostic snapshot (`export`), an offline query over it
//! (`lookup`), and the Uniswap token-list rendering of the Tokens list
//! (`tokenlist`). See `intend::lgtcr` for what is proven and what rests on
//! log agreement.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use alloy::primitives::{Address, B256};
use clap::{Args as ClapArgs, Parser, Subcommand};
use eyre::{bail, Context, Result};

use intend::anchor::ChainPin;
use intend::lgtcr::{
    self, Counts, ExportConfig, ListSnapshot, Provenance, StatusFilter, TokenList,
};

const DEFAULT_GENESIS: &str = "0x4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756";

#[derive(Parser, Debug)]
#[command(
    name = "curate-export",
    version,
    about = "Verified exports of Kleros Light Curate lists"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Snapshot a Light list: NewItem logs from two agreeing RPCs, status
    /// proofs in contract storage at a header-quorum anchor, hash-verified
    /// item files; writes items.json (+ optional CSV) and provenance.json.
    Export(ExportArgs),
    /// Render the Tokens list as a Uniswap-schema token list, from an
    /// items.json or by exporting first.
    Tokenlist(TokenlistArgs),
    /// Query an items.json offline: by address, by value substring, by status.
    Lookup(LookupArgs),
}

/// Where the data comes from: a preset or an explicit list, the sources.
#[derive(ClapArgs, Debug, Clone)]
struct SourceArgs {
    /// A known registry: tokens, address-tags, atq, cdn (fills --list, --items-slot, --from-block).
    #[arg(long)]
    registry: Option<String>,
    /// Any Light Curate list address (overrides the preset's).
    #[arg(long)]
    list: Option<String>,
    /// Chain id every RPC must serve.
    #[arg(long, default_value_t = 100)]
    chain_id: u64,
    /// Genesis block hash every RPC must serve.
    #[arg(long, default_value = DEFAULT_GENESIS)]
    genesis: String,
    /// Storage slot of the list's `items` mapping (10 for the Scout registries).
    #[arg(long)]
    items_slot: Option<u64>,
    /// Expected runtime code hash of the list (recorded, not pinned, when absent).
    #[arg(long)]
    code_hash: Option<String>,
    /// Header-quorum sources (at least two, independently operated).
    #[arg(long = "anchor-rpc", default_values_t = ["https://rpc.gnosischain.com".to_string(), "https://gnosis-rpc.publicnode.com".to_string()])]
    anchor_rpcs: Vec<String>,
    /// Log sources whose NewItem sets must agree (at least two). The defaults
    /// serve historical logs in million-block windows; PublicNode and dRPC cap
    /// a request at 10,000 blocks and work too, two thousand calls slower.
    #[arg(long = "log-rpc", default_values_t = ["https://rpc.gnosischain.com".to_string(), "https://gnosis.gateway.tenderly.co".to_string()])]
    log_rpcs: Vec<String>,
    /// Untrusted proof provider (must serve eth_getProof at the anchor).
    #[arg(long, default_value = "https://gnosis-rpc.publicnode.com")]
    provider_rpc: String,
    /// IPFS gateways for raw blocks (every block is hash-verified, so any
    /// gateway is safe); tried healthiest-first. The defaults answer path-style
    /// requests directly; dweb.link and ipfs.io redirect twice per block.
    #[arg(long = "gateway", default_values_t = ["https://trustless-gateway.net".to_string(), "https://cdn.kleros.link".to_string(), "https://ipfs.filebase.io".to_string(), "https://dweb.link".to_string(), "https://ipfs.io".to_string()])]
    gateways: Vec<String>,
    /// First block to scan for NewItem logs (preset's creation block, else discovered).
    #[arg(long)]
    from_block: Option<u64>,
    /// Log window in blocks (halved on RPC failure).
    #[arg(long, default_value_t = 1_000_000)]
    log_window: u64,
    /// Concurrent item fetches.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// Also include RegistrationRequested items (pending review or challenge).
    #[arg(long)]
    include_pending: bool,
    /// Include every status, Absent items with only the path their log carried.
    #[arg(long)]
    all: bool,
}

impl SourceArgs {
    fn config(&self) -> Result<ExportConfig> {
        let preset = match &self.registry {
            Some(name) => Some(lgtcr::preset(name).ok_or_else(|| {
                eyre::eyre!(
                    "unknown --registry {name:?}; known: {}",
                    lgtcr::PRESETS
                        .iter()
                        .map(|p| p.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?),
            None => None,
        };
        let list: Address = match (&self.list, preset) {
            (Some(l), _) => l.parse().wrap_err("--list")?,
            (None, Some(p)) => p.list,
            (None, None) => bail!("give --registry <name> or --list <address>"),
        };
        let items_slot = self
            .items_slot
            .or(preset.map(|p| p.items_slot))
            .unwrap_or(10);
        let from_block = self.from_block.or(preset.map(|p| p.from_block));
        Ok(ExportConfig {
            pin: ChainPin {
                chain_id: self.chain_id,
                genesis_hash: self.genesis.parse().wrap_err("--genesis")?,
            },
            list,
            registry: preset.map(|p| p.name.to_string()),
            items_slot,
            code_hash: match &self.code_hash {
                Some(h) => Some(h.parse::<B256>().wrap_err("--code-hash")?),
                None => None,
            },
            anchor_rpcs: self.anchor_rpcs.clone(),
            log_rpcs: self.log_rpcs.clone(),
            provider_rpc: self.provider_rpc.clone(),
            gateways: self.gateways.clone(),
            from_block,
            log_window: self.log_window,
            concurrency: self.concurrency,
            filter: StatusFilter {
                pending: self.include_pending,
                all: self.all,
            },
        })
    }
}

#[derive(ClapArgs, Debug)]
struct ExportArgs {
    #[command(flatten)]
    source: SourceArgs,
    #[arg(long, default_value = "items.json")]
    out: PathBuf,
    #[arg(long, default_value = "provenance.json")]
    provenance: PathBuf,
    /// Also write a CSV with one column per label.
    #[arg(long)]
    csv: Option<PathBuf>,
}

#[derive(ClapArgs, Debug)]
struct TokenlistArgs {
    /// A snapshot written by `export` (otherwise export runs first with the source flags).
    #[arg(long)]
    items: Option<PathBuf>,
    #[command(flatten)]
    source: SourceArgs,
    /// Where the snapshot goes when export runs here.
    #[arg(long, default_value = "items.json")]
    items_out: PathBuf,
    #[arg(long, default_value = "provenance.json")]
    provenance: PathBuf,
    /// Also include tokens whose removal is pending (ClearingRequested).
    #[arg(long)]
    include_clearing: bool,
    /// Previous export, for token-lists version semantics.
    #[arg(long)]
    previous: Option<PathBuf>,
    /// Also write the skipped items (item id and reason) as JSON.
    #[arg(long)]
    skipped_out: Option<PathBuf>,
    /// A reference list (URL or file) to diff against.
    #[arg(long)]
    compare: Option<String>,
    #[arg(long, default_value = "diff.json")]
    diff_out: PathBuf,
    /// Prefix for logo URIs (the item's /ipfs/ path follows).
    #[arg(long, default_value = "ipfs://")]
    logo_base: String,
    #[arg(long, default_value = "Kleros Tokens, verified export")]
    name: String,
    #[arg(long, default_value = "tokens.json")]
    out: PathBuf,
}

#[derive(ClapArgs, Debug)]
struct LookupArgs {
    #[arg(long, default_value = "items.json")]
    items: PathBuf,
    /// A 20-byte address; matches any address or eip155:<chain>:<address> value.
    #[arg(long)]
    address: Option<String>,
    /// Case-insensitive substring over every value.
    #[arg(long)]
    value: Option<String>,
    /// Status number (0 absent, 1 registered, 2 registrationRequested, 3 clearingRequested).
    #[arg(long)]
    status: Option<u8>,
    /// Print matches as JSON instead of text.
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Export(a) => export(a).await,
        Cmd::Tokenlist(a) => tokenlist(a).await,
        Cmd::Lookup(a) => lookup(a),
    }
}

fn command_line() -> String {
    std::env::args().collect::<Vec<_>>().join(" ")
}

/// Run the verified export and write the snapshot and its provenance.
async fn run_export(
    cfg: &ExportConfig,
    out: &Path,
    provenance: &Path,
    csv: Option<&Path>,
) -> Result<(lgtcr::ExportOutcome, Vec<u8>)> {
    let started = std::time::Instant::now();
    let mut progress = |line: String| eprintln!("{line}");
    let outcome = lgtcr::export_list(cfg, &mut progress).await?;
    let bytes = outcome.snapshot.to_bytes()?;
    intend::store::atomic_write(out, &bytes)?;
    if let Some(csv_path) = csv {
        intend::store::atomic_write(csv_path, lgtcr::snapshot_csv(&outcome.snapshot).as_bytes())?;
    }
    let skipped: Vec<lgtcr::Skipped> = outcome
        .snapshot
        .items
        .iter()
        .filter_map(|it| {
            it.error.as_ref().map(|e| lgtcr::Skipped {
                item_id: it.item_id.clone(),
                reason: e.clone(),
            })
        })
        .collect();
    let prov = Provenance {
        chain_id: cfg.pin.chain_id,
        genesis: format!("{}", cfg.pin.genesis_hash),
        list: cfg.list.to_checksum(None),
        registry: cfg.registry.clone(),
        items_slot: cfg.items_slot,
        code_hash: format!("{}", outcome.code_hash),
        anchor: outcome.snapshot.anchor.clone(),
        anchor_rpcs: cfg.anchor_rpcs.clone(),
        log_rpcs: cfg.log_rpcs.clone(),
        provider_rpc: cfg.provider_rpc.clone(),
        gateways: cfg.gateways.clone(),
        from_block: outcome.from_block,
        from_block_source: outcome.from_block_source.clone(),
        included_statuses: outcome.snapshot.included_statuses.clone(),
        counts: Counts {
            logs: outcome.logs,
            unique_items: outcome.unique_items,
            by_status: outcome.by_status.clone(),
            included: outcome.snapshot.items.len() as u64,
            skipped: skipped.len() as u64,
        },
        skipped,
        output_sha256: lgtcr::sha256_hex(&bytes),
        rpc_calls: outcome.rpc_calls,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        command_line: command_line(),
        note: format!(
            "Membership and status are proven against the anchor's state root; item content is bound by hash. \
             Completeness rests on the agreement of the {} log sources and on the header quorum ({}); \
             {} path/id mismatches were dropped.",
            cfg.log_rpcs.len(),
            outcome.anchor_mode,
            outcome.mismatched
        ),
    };
    let mut prov_bytes = serde_json::to_vec_pretty(&prov)?;
    prov_bytes.push(b'\n');
    intend::store::atomic_write(provenance, &prov_bytes)?;
    Ok((outcome, bytes))
}

async fn export(a: ExportArgs) -> Result<()> {
    let cfg = a.source.config()?;
    let (outcome, bytes) = run_export(&cfg, &a.out, &a.provenance, a.csv.as_deref()).await?;
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "registry": cfg.registry,
            "list": cfg.list.to_checksum(None),
            "anchorBlock": outcome.anchor.block_number,
            "anchorBlockHash": outcome.anchor.block_hash,
            "anchorMode": outcome.anchor_mode,
            "fromBlock": outcome.from_block,
            "logs": outcome.logs,
            "uniqueItems": outcome.unique_items,
            "byStatus": outcome.by_status,
            "included": outcome.snapshot.items.len(),
            "fetchFailures": outcome.fetch_failures,
            "rpcCalls": outcome.rpc_calls,
            "out": a.out.display().to_string(),
            "outputSha256": lgtcr::sha256_hex(&bytes),
            "provenance": a.provenance.display().to_string(),
            "csv": a.csv.as_ref().map(|p| p.display().to_string()),
        })
    );
    Ok(())
}

async fn tokenlist(a: TokenlistArgs) -> Result<()> {
    let snapshot: ListSnapshot = match &a.items {
        Some(path) => {
            let bytes =
                std::fs::read(path).wrap_err_with(|| format!("--items {}", path.display()))?;
            ListSnapshot::from_bytes(&bytes)?
        }
        None => {
            let mut source = a.source.clone();
            if source.registry.is_none() && source.list.is_none() {
                source.registry = Some("tokens".into());
            }
            let cfg = source.config()?;
            run_export(&cfg, &a.items_out, &a.provenance, None)
                .await?
                .0
                .snapshot
        }
    };
    let (tokens, skipped) =
        lgtcr::tokens_from_snapshot(&snapshot, a.include_clearing, &a.logo_base)?;
    let previous: Option<TokenList> = match &a.previous {
        Some(p) => {
            let bytes = std::fs::read(p).wrap_err_with(|| format!("--previous {}", p.display()))?;
            Some(serde_json::from_slice(&bytes).wrap_err("--previous is not a token list")?)
        }
        None => None,
    };
    let list_out = TokenList {
        name: a.name.clone(),
        timestamp: lgtcr::rfc3339(snapshot.anchor.timestamp),
        version: lgtcr::bump_version(previous.as_ref(), &tokens),
        tokens: tokens.clone(),
    };
    let bytes = lgtcr::serialize_list(&list_out)?;
    intend::store::atomic_write(&a.out, &bytes)?;
    if let Some(path) = &a.skipped_out {
        let mut skipped_bytes = serde_json::to_vec_pretty(&skipped)?;
        skipped_bytes.push(b'\n');
        intend::store::atomic_write(path, &skipped_bytes)?;
    }

    let mut diff_summary = serde_json::Value::Null;
    if let Some(reference) = &a.compare {
        let ref_bytes = if reference.starts_with("http://") || reference.starts_with("https://") {
            intend::fetch::fetch_bounded(
                reference,
                lgtcr::MAX_REFERENCE_BYTES,
                Some("application/json"),
            )
            .await?
        } else {
            std::fs::read(reference).wrap_err_with(|| format!("--compare {reference}"))?
        };
        let theirs = lgtcr::parse_reference_list(&ref_bytes)?;
        let diff = lgtcr::diff_lists(&tokens, &theirs);
        let mut diff_bytes = serde_json::to_vec_pretty(&diff)?;
        diff_bytes.push(b'\n');
        intend::store::atomic_write(&a.diff_out, &diff_bytes)?;
        diff_summary = serde_json::json!({
            "reference": reference,
            "referenceTokens": theirs.len(),
            "oursOnly": diff.ours_only.len(),
            "theirsOnly": diff.theirs_only.len(),
            "changed": diff.changed.len(),
            "diffFile": a.diff_out.display().to_string(),
        });
    }
    let mut skip_reasons: BTreeMap<String, u64> = BTreeMap::new();
    for s in &skipped {
        let key = s.reason.split(':').next().unwrap_or("").to_string();
        *skip_reasons.entry(key).or_default() += 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "anchorBlock": snapshot.anchor.number,
            "anchorMode": snapshot.anchor.mode,
            "tokens": tokens.len(),
            "skipped": skipped.len(),
            "skipReasons": skip_reasons,
            "skippedOut": a.skipped_out.as_ref().map(|p| p.display().to_string()),
            "version": list_out.version,
            "out": a.out.display().to_string(),
            "tokensSha256": lgtcr::sha256_hex(&bytes),
            "compare": diff_summary,
        })
    );
    Ok(())
}

fn lookup(a: LookupArgs) -> Result<()> {
    let bytes =
        std::fs::read(&a.items).wrap_err_with(|| format!("--items {}", a.items.display()))?;
    let snapshot = ListSnapshot::from_bytes(&bytes)?;
    let q = lgtcr::Lookup {
        address: match &a.address {
            Some(s) => Some(s.parse::<Address>().wrap_err("--address")?),
            None => None,
        },
        value: a.value.clone(),
        status: a.status,
    };
    if q.address.is_none() && q.value.is_none() && q.status.is_none() {
        bail!("give at least one of --address, --value, --status");
    }
    let hits = lgtcr::lookup(&snapshot, &q);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
    } else {
        println!(
            "{} of {} items match (list {}, anchor block {}, {})",
            hits.len(),
            snapshot.items.len(),
            snapshot.list,
            snapshot.anchor.number,
            snapshot.anchor.mode
        );
        for it in hits {
            println!("- {} [{}] {}", it.item_id, it.status_name, it.path);
            if let Some(map) = it.values.as_object() {
                for (k, v) in map {
                    let text = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if !text.is_empty() {
                        println!("    {k}: {text}");
                    }
                }
            }
        }
    }
    Ok(())
}
