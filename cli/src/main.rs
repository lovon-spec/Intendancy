//! `intend` — Intendhub consumer CLI (pre-1.0).
//!
//! Anchor strength, enumeration scope, and freshness are reported SEPARATELY on
//! every command (repo invariant): this build anchors via header quorum only
//! (degraded alpha; every source authenticated against the pinned chainId +
//! genesis), snapshots are complete enumerations, and installs/audits do fresh
//! point checks (verified, non-exhaustive) at the latest finalized quorum
//! header. State read-modify-write is serialized with interprocess locks.

use std::path::{Path, PathBuf};

use alloy::primitives::B256;
use clap::{Parser, Subcommand};
use eyre::{bail, Context, Result};

use intend::anchor::{self, MODE_LABEL};
use intend::car;
use intend::chain;
use intend::fetch;
use intend::lockfile::{verify_local_integrity, AuditRecord, Entry, Lockfile};
use intend::policy;
use intend::profile::Profile;
use intend::schema::{screen, Descriptor};
use intend::snapshot::{self, Limits, Snapshot};
use intend::store::{
    lockfile_lock_path, state_lock_path, Catalog, CatalogMeta, HighWater, ScopeLock,
};

/// Maximum snapshot age accepted by `update`, measured against the
/// AUTHENTICATED header timestamp. PROVISIONAL working default — the production
/// value is a pre-launch owner decision (spec §8/§11).
const MAX_SNAPSHOT_AGE_SECS: u64 = 24 * 3600;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path to the pinned registry profile (TOML; spec §3).
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    /// Local state directory (default: $INTEND_STATE_DIR or ~/.intend).
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Fetch/verify a COMPLETE registry snapshot at a quorum-authenticated
    /// finalized anchor and persist it as the local catalog. Sources, in
    /// order: --snapshot file, the profile's snapshot_urls (failover continues
    /// past verification failures), else self-generation via provider_rpc.
    Update {
        /// Verify a provider-supplied snapshot file (raw JSON or .gz).
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// List the verified catalog.
    Catalog,
    /// Install one skill by name or 0x-itemID: fresh point check, CAR fetch
    /// with gateway failover, full DAG verification against the descriptor's
    /// Tree CID plus the SKILL.md byte-binding, staged atomic no-replace
    /// publish, lockfile entry (lockfile proven writable BEFORE publish).
    Install {
        /// Skill name (unique, Registered) or 0x-prefixed itemID.
        item: String,
        /// Destination directory (must not exist).
        #[arg(long)]
        dir: PathBuf,
        /// Read the CAR from a local file instead of the profile's gateways.
        #[arg(long)]
        car: Option<PathBuf>,
        /// Lockfile path (default ./intend-lock.json).
        #[arg(long)]
        lockfile: Option<PathBuf>,
    },
    /// Fresh-check every lockfile entry AND verify installed bytes; record
    /// current / reenable-required / quarantined / revoked / blocked /
    /// modified states (spec §8; quarantine and revocation are STICKY).
    Audit {
        /// Lockfile path (default ./intend-lock.json).
        #[arg(long)]
        lockfile: Option<PathBuf>,
    },
    /// Explicitly clear a sticky suspension for one installed entry, after a
    /// fresh Registered check and a local integrity pass (spec §8's re-enable
    /// transition).
    Enable {
        /// The entry's install directory (as recorded in the lockfile).
        dir: String,
        /// Lockfile path (default ./intend-lock.json).
        #[arg(long)]
        lockfile: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let profile_path = args
        .profile
        .clone()
        .ok_or_else(|| eyre::eyre!("--profile <file.toml> is required"))?;
    let profile = Profile::load(&profile_path)?;
    let state_dir = match &args.state_dir {
        Some(d) => d.clone(),
        None => intend::store::default_state_dir()?,
    };
    let limits = Limits::default();

    match args.cmd {
        Cmd::Update { snapshot: file } => {
            let _state_lock = ScopeLock::acquire(&state_lock_path(&state_dir))?;
            // ONE candidate at a time: fetch → parse → verify → persist-or-DISCARD
            // before the next source is contacted (review finding: never download
            // or retain all candidates).
            let mut errors: Vec<String> = Vec::new();
            let mut done = false;
            if let Some(path) = file {
                let source = format!("file {}", path.display());
                let snap = snapshot::read_snapshot_file_bounded(&path, &limits).map(|(s, _)| s)?;
                verify_and_persist(&profile, &state_dir, &limits, snap, &source).await?;
                done = true;
            } else if !profile.snapshot_urls.is_empty() {
                for url in &profile.snapshot_urls {
                    let source = format!("url {url}");
                    let attempt = async {
                        let bytes = fetch::fetch_snapshot(url, &limits).await?;
                        let (snap, _) = snapshot::read_snapshot_bounded(&bytes, &limits)?;
                        drop(bytes);
                        verify_and_persist(&profile, &state_dir, &limits, snap, &source).await
                    }
                    .await;
                    match attempt {
                        Ok(()) => {
                            done = true;
                            break;
                        }
                        Err(e) => errors.push(format!("{source}: {e:#}")),
                    }
                }
            } else {
                let rpc = profile.proof_rpc().to_string();
                let source = format!("self-generated via {rpc}");
                let quorum = anchor::finalized_quorum(&profile).await?;
                let (quorum, _) = finish_anchor(&profile, quorum).await?;
                let snap = chain::generate_snapshot(&rpc, &profile, &quorum, &limits).await?;
                verify_and_persist(&profile, &state_dir, &limits, snap, &source).await?;
                done = true;
            }
            if !done {
                bail!(
                    "no snapshot source verified. Attempts:\n{}",
                    errors.join("\n")
                );
            }
        }

        Cmd::Catalog => {
            let catalog = Catalog::load(&state_dir)?;
            catalog.assert_bound_to(
                profile.chain_id,
                profile.registry,
                profile.deployment_context_id(),
            )?;
            println!(
                "# {} items @ block {} ({}), verified {}",
                catalog.meta.items,
                catalog.meta.anchor_block,
                catalog.meta.anchor_mode,
                catalog.meta.verified_at_unix
            );
            for row in &catalog.snapshot.rows {
                let d = Descriptor::decode(&row.descriptor)?;
                println!(
                    "{}  {}  {}  {}",
                    status_name(row.status),
                    row.item_id,
                    d.name,
                    d.tree_cid
                );
            }
        }

        Cmd::Install {
            item,
            dir,
            car: car_file,
            lockfile: lock_path,
        } => {
            let _state_lock = ScopeLock::acquire(&state_lock_path(&state_dir))?;
            // ONE stable lockfile identity BEFORE the data path and its
            // companion lock are derived (round-5): symlink aliases converge.
            let lock_path = intend::lockfile::resolve_lockfile_path(
                &lock_path.unwrap_or_else(intend::lockfile::default_lockfile_path),
            )?;
            let _lockfile_lock = ScopeLock::acquire(&lockfile_lock_path(&lock_path))?;

            let catalog = Catalog::load(&state_dir)?;
            // Name/completeness resolution must come from THIS profile's
            // registry, not whatever the state dir last verified (round-5).
            catalog.assert_bound_to(
                profile.chain_id,
                profile.registry,
                profile.deployment_context_id(),
            )?;
            let (row, descriptor) = resolve_item(&catalog, &item)?;
            screen(&descriptor)?;

            // Load AND prove the lockfile writable BEFORE any content publish
            // (review finding 4): a malformed or unwritable lockfile must fail
            // the command while the filesystem is still untouched.
            let mut lock = Lockfile::load(&lock_path)?;
            lock.save(&lock_path)
                .wrap_err("lockfile is not writable — refusing to install")?;

            // Fresh point check at the LATEST finalized quorum anchor (spec §8).
            let quorum = anchor::finalized_quorum(&profile).await?;
            let (quorum, mode_label) = finish_anchor(&profile, quorum).await?;
            let mut hw = HighWater::load(&state_dir)?;
            hw.observe(
                profile.chain_id,
                profile.genesis_hash,
                profile.registry,
                &quorum,
            )?;
            let status =
                chain::point_check(profile.proof_rpc(), &profile, &quorum, row.item_id).await?;
            if status != 1 {
                bail!(
                    "item {} is {} at block {} — only Registered (1) installs (failing closed)",
                    row.item_id,
                    status_name(status),
                    quorum.block_number
                );
            }

            // Acquire VERIFIED blocks + the verified install PLAN: each source
            // attempt runs the full parse + preflight + SKILL.md-binding
            // validation; failover continues past verification failures
            // (review finding 9). The returned plan is the complete manifest
            // the journal below persists.
            let expected = car::Cid::parse_canonical(&descriptor.tree_cid)?;
            let (plan, blocks) =
                acquire_verified_blocks(&profile, &descriptor, expected, car_file.as_deref())
                    .await?;

            // INSTALL TRANSACTION (round-3 P0s). Order:
            //   1. BIND the destination once (canonical parent held as a fd) —
            //      journal and publish share this one binding.
            //   2. CHECK destination occupancy AND any existing lockfile record
            //      BEFORE journaling: a repeat install fails here with the
            //      prior record untouched (insert_new never replaces).
            //   3. JOURNAL the pending entry WITH THE FULL VERIFIED MANIFEST
            //      (files/dirs/digests/totals) and persist it.
            //   4. PUBLISH (staged, fsynced, atomic no-replace, fd-relative).
            //   5. FINALIZE by clearing the pending flag. If the finalizing
            //      save fails, content and the full-manifest journal both
            //      stand (published content is never deleted): `intend audit`
            //      reports the entry "incomplete" and `intend enable <dir>`
            //      finalizes it after a fresh Registered proof + an integrity
            //      pass against the journaled manifest.
            let target = car::InstallTarget::bind(&dir)?;
            target.check_vacant()?;
            let dirs: Vec<String> = plan
                .dirs
                .iter()
                .map(|d| {
                    d.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| eyre::eyre!("plan path {d:?} is not UTF-8"))
                })
                .collect::<Result<_>>()?;
            let mut entry = Entry {
                name: descriptor.name.clone(),
                item_id: row.item_id,
                tree_cid: descriptor.tree_cid.clone(),
                chain_id: profile.chain_id,
                registry: profile.registry,
                deployment_context_id: profile.deployment_context_id(),
                install_dir: target.path_str().to_string(),
                installed_at_unix: unix_now()?,
                status_at_install: status,
                anchor_block: quorum.block_number,
                anchor_block_hash: quorum.block_hash,
                anchor_state_root: quorum.state_root,
                root_block_sha256: car::hex_lower(&expected.digest),
                total_bytes: plan.total_bytes,
                blocks: plan.blocks,
                files: plan.locked_files(),
                dirs,
                audit: None,
                sticky_suspension: None,
                pending: true,
            };
            lock.insert_new(entry.clone())?;
            lock.save(&lock_path).wrap_err(
                "journaling the pending install failed — no content was published \
                 (the journal write itself may or may not have become durable; a \
                 surviving entry is reported by `intend audit`)",
            )?;

            let published = car::publish(&plan, &blocks, &target)?;
            // Never KNOWINGLY finalize a stale identity (round-4): if the
            // parent pathname was retargeted after the publish, the journaled
            // path no longer resolves to the published content — leave the
            // entry pending (fail-closed; audit reports it) instead of
            // finalizing a record that points elsewhere.
            target.verify_identity().wrap_err(
                "published, but the destination path no longer names the bound \
                 directory — the pending journal stands; run `intend audit`",
            )?;
            entry.pending = false;
            let report_files = entry.files.len();
            // Replaces exactly this transaction's own journal (both locks are
            // held across the whole transaction).
            lock.upsert(entry);
            lock.save(&lock_path).wrap_err(
                "content is PUBLISHED but the lockfile finalization failed; the \
                 pending journal (with the full manifest) stands — run \
                 `intend audit` then `intend enable` to finalize",
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "name": descriptor.name,
                    "itemId": row.item_id,
                    "treeCid": descriptor.tree_cid,
                    "installedTo": published.display().to_string(),
                    "files": report_files,
                    "materializedBytes": plan.total_bytes,
                    "freshStatus": "Registered",
                    "freshCheck": "verified, non-exhaustive (point mode)",
                    "anchorMode": mode_label,
                    "anchorBlock": quorum.block_number,
                    "lockfile": lock_path.display().to_string(),
                })
            );
        }

        Cmd::Audit {
            lockfile: lock_path,
        } => {
            let _state_lock = ScopeLock::acquire(&state_lock_path(&state_dir))?;
            let lock_path = intend::lockfile::resolve_lockfile_path(
                &lock_path.unwrap_or_else(intend::lockfile::default_lockfile_path),
            )?;
            let _lockfile_lock = ScopeLock::acquire(&lockfile_lock_path(&lock_path))?;
            let mut lock = Lockfile::load(&lock_path)?;
            if lock.entries.is_empty() {
                println!("lockfile {} has no entries", lock_path.display());
                return Ok(());
            }
            let quorum = anchor::finalized_quorum(&profile).await?;
            let (quorum, mode_label) = finish_anchor(&profile, quorum).await?;
            let mut hw = HighWater::load(&state_dir)?;
            hw.observe(
                profile.chain_id,
                profile.genesis_hash,
                profile.registry,
                &quorum,
            )?;
            let now = unix_now()?;
            let mut all_current = true;
            let mut report = Vec::new();
            for entry in &mut lock.entries {
                if entry.chain_id != profile.chain_id
                    || entry.registry != profile.registry
                    || entry.deployment_context_id != profile.deployment_context_id()
                {
                    bail!(
                        "lockfile entry {} is bound to {}:{} (context {}) — not this \
                         profile's deployment context",
                        entry.name,
                        entry.chain_id,
                        entry.registry,
                        entry.deployment_context_id
                    );
                }
                // Local integrity FIRST (review finding 3): the installed bytes
                // must match the lockfile exactly.
                let integrity = verify_local_integrity(entry);
                let status =
                    chain::point_check(profile.proof_rpc(), &profile, &quorum, entry.item_id)
                        .await?;
                let (state, sticky) =
                    intend::lockfile::audit_state_for(entry, status, integrity.is_ok());
                if state != "current" {
                    all_current = false;
                }
                entry.sticky_suspension = sticky;
                entry.audit = Some(AuditRecord {
                    checked_at_unix: now,
                    anchor_block: quorum.block_number,
                    anchor_block_hash: quorum.block_hash,
                    status,
                    state: state.into(),
                });
                report.push(serde_json::json!({
                    "name": entry.name,
                    "itemId": entry.item_id,
                    "installDir": entry.install_dir,
                    "freshStatus": status_name(status),
                    "localIntegrity": match &integrity {
                        Ok(()) => "intact".to_string(),
                        Err(e) => format!("{e:#}"),
                    },
                    "state": state,
                }));
            }
            lock.save(&lock_path)?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": all_current,
                    "anchorMode": mode_label,
                    "freshCheck": "verified, non-exhaustive (point mode per entry)",
                    "anchorBlock": quorum.block_number,
                    "entries": report,
                })
            );
            if !all_current {
                std::process::exit(1);
            }
        }

        Cmd::Enable {
            dir,
            lockfile: lock_path,
        } => {
            let _state_lock = ScopeLock::acquire(&state_lock_path(&state_dir))?;
            let lock_path = intend::lockfile::resolve_lockfile_path(
                &lock_path.unwrap_or_else(intend::lockfile::default_lockfile_path),
            )?;
            let _lockfile_lock = ScopeLock::acquire(&lockfile_lock_path(&lock_path))?;
            let mut lock = Lockfile::load(&lock_path)?;
            // Match by canonical path so aliases and cwd differences still
            // find the entry (entries store canonical paths).
            let wanted = std::path::Path::new(&dir)
                .canonicalize()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| dir.clone());
            let entry = lock
                .entries
                .iter_mut()
                .find(|e| e.install_dir == wanted || e.install_dir == dir)
                .ok_or_else(|| eyre::eyre!("no lockfile entry installed at {dir:?}"))?;
            let quorum = anchor::finalized_quorum(&profile).await?;
            let (quorum, mode_label) = finish_anchor(&profile, quorum).await?;
            let mut hw = HighWater::load(&state_dir)?;
            hw.observe(
                profile.chain_id,
                profile.genesis_hash,
                profile.registry,
                &quorum,
            )?;
            let status =
                chain::point_check(profile.proof_rpc(), &profile, &quorum, entry.item_id).await?;
            // The ONE production enable/finalize path (guards, manifest rule,
            // integrity, Registered requirement, pending-destination
            // durability fsync) — shared verbatim with the crash-recovery
            // tests (round-4).
            let cleared =
                intend::lockfile::enable_transition(entry, &profile.proof_context(), status)
                    .wrap_err_with(|| {
                        format!("cannot enable {dir:?} at block {}", quorum.block_number)
                    })?;
            entry.audit = Some(AuditRecord {
                checked_at_unix: unix_now()?,
                anchor_block: quorum.block_number,
                anchor_block_hash: quorum.block_hash,
                status,
                state: "current".into(),
            });
            let name = entry.name.clone();
            lock.save(&lock_path)?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "name": name,
                    "installDir": dir,
                    "clearedSuspension": cleared,
                    "freshStatus": "Registered",
                    "anchorMode": mode_label,
                    "anchorBlock": quorum.block_number,
                })
            );
        }
    }
    Ok(())
}

/// Full §6/§8 verification of one candidate snapshot and, on success, catalog +
/// high-water persistence and the success report.
async fn verify_and_persist(
    profile: &Profile,
    state_dir: &Path,
    limits: &Limits,
    snap: Snapshot,
    source: &str,
) -> Result<()> {
    // Quorum-authenticate the snapshot's declared anchor height.
    let quorum = anchor::quorum_anchor_at(profile, snap.anchor.block_number).await?;
    let (quorum, mode_label) = finish_anchor(profile, quorum).await?;
    let now = unix_now()?;
    let age = now.saturating_sub(quorum.timestamp);
    if age > MAX_SNAPSHOT_AGE_SECS {
        bail!(
            "snapshot anchor is {age}s old (> {MAX_SNAPSHOT_AGE_SECS}s) — obtain a fresher \
             snapshot (spec §8 maximum snapshot age; value provisional pre-launch)"
        );
    }
    let vp = profile.verifier_profile(&quorum);
    let stats = snapshot::verify(&snap, &vp, limits)?;
    let mut status_counts = [0u64; 4];
    for (i, row) in snap.rows.iter().enumerate() {
        let d = Descriptor::decode(&row.descriptor).map_err(|e| eyre::eyre!("row {i}: {e}"))?;
        screen(&d).map_err(|e| eyre::eyre!("row {i} fails policy screening: {e}"))?;
        status_counts[row.status as usize] += 1;
    }
    let mut hw = HighWater::load(state_dir)?;
    hw.observe(
        profile.chain_id,
        profile.genesis_hash,
        profile.registry,
        &quorum,
    )?;
    let raw = serde_json::to_vec(&snap)?;
    use sha2::{Digest, Sha256};
    let meta = CatalogMeta {
        deployment_context_id: profile.deployment_context_id(),
        verified_at_unix: now,
        anchor_block: quorum.block_number,
        anchor_block_hash: quorum.block_hash,
        anchor_state_root: quorum.state_root,
        anchor_mode: mode_label.clone(),
        items: stats.items,
        snapshot_sha256: car::hex_lower(&Sha256::digest(&raw)),
    };
    Catalog::save(state_dir, &meta, &snap)?;
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "source": source,
            "anchorMode": mode_label,
            "enumeration": "complete (length + contiguous itemList + statuses proven)",
            "anchorBlock": quorum.block_number,
            "anchorBlockHash": quorum.block_hash,
            "anchorAgeSecs": age,
            "quorumSources": quorum.sources,
            "items": stats.items,
            "statusCounts": {
                "absent": status_counts[0],
                "registered": status_counts[1],
                "registrationRequested": status_counts[2],
                "clearingRequested": status_counts[3],
            },
            "slotProofsChecked": stats.slot_proofs_checked,
        })
    );
    Ok(())
}

/// Obtain the verified INSTALL PLAN plus its block map — read_car + preflight +
/// the SKILL.md binding — from the --car file or the profile's gateways in
/// order. A failure at ANY stage moves on to the next source. The plan is the
/// complete manifest the install transaction journals before publishing.
async fn acquire_verified_blocks(
    profile: &Profile,
    descriptor: &Descriptor,
    expected: car::Cid,
    car_file: Option<&Path>,
) -> Result<(
    car::InstallPlan,
    std::collections::BTreeMap<car::Cid, Vec<u8>>,
)> {
    let validate = |bytes: &[u8]| -> Result<(
        car::InstallPlan,
        std::collections::BTreeMap<car::Cid, Vec<u8>>,
    )> {
        let (_claimed, blocks) = car::read_car(bytes)?;
        let plan = car::preflight(expected, &blocks)?;
        policy::verify_skill_binding(&plan, &blocks, descriptor)?;
        Ok((plan, blocks))
    };
    if let Some(path) = car_file {
        let bytes = bounded_file_read(path, car::MAX_CAR_BYTES)?;
        return validate(&bytes)
            .wrap_err_with(|| format!("CAR file {} failed verification", path.display()));
    }
    if profile.gateways.is_empty() {
        bail!("no IPFS gateways configured in the profile (and no --car file given)");
    }
    let mut errors = Vec::new();
    for gw in &profile.gateways {
        match fetch::fetch_car(gw, &descriptor.tree_cid).await {
            Ok(bytes) => match validate(&bytes) {
                Ok(verified) => return Ok(verified),
                Err(e) => errors.push(format!("{gw}: verification failed: {e:#}")),
            },
            Err(e) => errors.push(format!("{gw}: fetch failed: {e:#}")),
        }
    }
    bail!("no gateway served a verifiable CAR:\n{}", errors.join("\n"))
}

/// Finish an anchor for use: real chains carry the state root in the header and
/// pass through; fork/dev chains (anvil) serve ZERO state roots and are refused
/// unless the profile's TEST-ONLY workaround is set, which derives the root from
/// a provider probe proof and DEGRADES the reported anchor mode accordingly.
async fn finish_anchor(
    profile: &Profile,
    mut quorum: intend::anchor::QuorumAnchor,
) -> Result<(intend::anchor::QuorumAnchor, String)> {
    if quorum.state_root == B256::ZERO {
        if !profile.test_headerless_state_root {
            bail!(
                "quorum header at block {} carries a ZERO stateRoot (fork/dev chain?). \
                 Refusing: the anchor cannot be authenticated. For TEST chains only, \
                 set test_headerless_state_root = true in the profile.",
                quorum.block_number
            );
        }
        quorum.state_root =
            chain::probe_state_root(profile.proof_rpc(), profile, quorum.block_number).await?;
        Ok((
            quorum,
            format!(
                "{MODE_LABEL} + TEST headerless-state-root workaround \
                 (state root is PROVIDER-derived, not header-authenticated)"
            ),
        ))
    } else if profile.test_headerless_state_root {
        bail!(
            "test_headerless_state_root is set but this chain serves real header state \
             roots — remove the workaround from the profile"
        );
    } else {
        Ok((quorum, MODE_LABEL.to_string()))
    }
}

fn unix_now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| eyre::eyre!("system clock before epoch: {e}"))?
        .as_secs())
}

fn status_name(status: u8) -> &'static str {
    match status {
        0 => "Absent",
        1 => "Registered",
        2 => "RegistrationRequested",
        3 => "ClearingRequested",
        _ => "undecodable",
    }
}

/// Resolve a user reference (0x-itemID or unique Registered name) against the
/// verified catalog.
fn resolve_item(catalog: &Catalog, reference: &str) -> Result<(intend::snapshot::Row, Descriptor)> {
    if let Some(hex) = reference.strip_prefix("0x") {
        let id: B256 = hex
            .parse::<B256>()
            .or_else(|_| format!("0x{hex}").parse())
            .wrap_err("itemID must be 32 bytes of hex")?;
        let row = catalog
            .snapshot
            .rows
            .iter()
            .find(|r| r.item_id == id)
            .ok_or_else(|| eyre::eyre!("itemID {id} is not in the verified catalog"))?;
        let d = Descriptor::decode(&row.descriptor)?;
        return Ok((row.clone(), d));
    }
    let mut matches = Vec::new();
    for row in &catalog.snapshot.rows {
        let d = Descriptor::decode(&row.descriptor)?;
        if d.name == reference {
            matches.push((row.clone(), d));
        }
    }
    match matches.len() {
        0 => bail!("no catalog entry named {reference:?}"),
        1 => Ok(matches.remove(0)),
        _ => {
            let mut registered: Vec<_> =
                matches.into_iter().filter(|(r, _)| r.status == 1).collect();
            match registered.len() {
                1 => Ok(registered.remove(0)),
                0 => bail!(
                    "name {reference:?} matches multiple entries, none currently Registered — \
                     install by 0x-itemID"
                ),
                _ => bail!(
                    "name {reference:?} is ambiguous among Registered entries — install by \
                     0x-itemID"
                ),
            }
        }
    }
}

/// Read a local file with a hard byte cap (streaming; never materializes more
/// than cap + 1 bytes).
fn bounded_file_read(path: &Path, cap: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).wrap_err_with(|| format!("opening {}", path.display()))?;
    let mut buf = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .wrap_err_with(|| format!("reading {}", path.display()))?;
    if buf.len() as u64 > cap {
        bail!("{} exceeds the {cap}-byte cap", path.display());
    }
    Ok(buf)
}
