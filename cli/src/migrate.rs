//! Policy-version migration of a lockfile (`intend migrate`; spec §3
//! `policyVersions`). A profile that accepts a newly announced policy version
//! has a different deployment context id, so every entry installed under the
//! previous profile is refused by `audit` and `enable` until it is carried
//! across explicitly: the new profile must be a strict SUCCESSOR of the old
//! one (`Profile::is_successor_of`), every entry must be bound to the old
//! context (or already to the new one, in which case it is skipped), and every
//! migrated entry gets a fresh point check UNDER THE NEW PROFILE plus a local
//! integrity verdict before it is rebound. Context rebinding is all-or-nothing;
//! an entry whose bytes are missing, moved or modified, or whose install never
//! finished, migrates in that recorded state without ever having its bytes
//! reported intact or enabled. When a migration aborts after some entries
//! were already proven, the adverse observations among them are persisted as
//! sticky suspensions — the same property the audit sweep keeps — so a later
//! return to Registered can never skip the explicit re-enable.
//!
//! The network-free parts live here so they are testable with fixtures: the
//! plan (which entries move), its application given the fresh checks, and the
//! safety persistence of a partial failure. `main.rs` supplies the anchor and
//! the point checks.

use alloy::primitives::B256;
use eyre::{bail, Result};

use crate::lockfile::{
    migrate_transition, record_observation, Entry, LocalVerdict, Lockfile, ProofContext,
};
use crate::profile::Profile;
use crate::store::{AtomicWriteFailure, WritePhase};

/// Which entries a migration carries across, decided before any network work.
#[derive(Debug)]
pub struct MigrationPlan {
    pub from: ProofContext,
    pub to: ProofContext,
    /// Indices (into `Lockfile::entries`) bound to the old context.
    pub to_migrate: Vec<usize>,
    /// Indices already bound to the new context — left untouched.
    pub skipped: Vec<usize>,
}

/// The local-integrity verdict of an entry's installed bytes, as the
/// migration records it. A failure is not a refusal: the entry migrates in a
/// `modified` state, its bytes unauthorized.
pub fn local_verdict(entry: &Entry) -> LocalVerdict {
    LocalVerdict::of(entry)
}

/// The fresh evidence the caller obtained for one entry under the NEW profile.
#[derive(Debug, Clone)]
pub struct Checked {
    pub index: usize,
    pub fresh_status: u8,
    pub local: LocalVerdict,
}

/// A status proven under the new profile before the migration aborted.
#[derive(Debug, Clone, Copy)]
pub struct Observed {
    pub index: usize,
    pub fresh_status: u8,
}

/// What one migrated entry recorded.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryResult {
    pub name: String,
    pub install_dir: String,
    pub fresh_status: u8,
    pub state: String,
    pub local_integrity: String,
    pub sticky_suspension: Option<String>,
}

/// What a completed migration recorded.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub migrated: usize,
    pub skipped: usize,
    pub current: usize,
    pub reenable_required: usize,
    pub quarantined: usize,
    pub revoked: usize,
    pub blocked: usize,
    pub modified: usize,
    pub incomplete: usize,
    pub entries: Vec<EntryResult>,
}

impl Summary {
    /// Entries that carry a sticky suspension after the migration.
    pub fn suspended(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.sticky_suspension.is_some())
            .count()
    }
}

/// An entry whose adverse observation was persisted by an aborted migration.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Affected {
    pub name: String,
    pub install_dir: String,
    pub fresh_status: u8,
    pub sticky_suspension: String,
}

/// Decide the plan: `new` must be a successor of `old`; each entry must be
/// bound to exactly one of the two contexts. Any other binding aborts the
/// whole migration by name, before anything is touched.
pub fn prepare(new: &Profile, old: &Profile, lock: &Lockfile) -> Result<MigrationPlan> {
    if let Err(reason) = new.is_successor_of(old) {
        bail!(
            "the active profile is not a successor of the --from profile: {reason}. A \
             migration carries entries across a policy-version transition only; anything \
             else is a different deployment or trust configuration"
        );
    }
    let from = old.proof_context();
    let to = new.proof_context();
    let mut plan = MigrationPlan {
        from,
        to,
        to_migrate: Vec::new(),
        skipped: Vec::new(),
    };
    for (i, entry) in lock.entries.iter().enumerate() {
        let bound_to = ProofContext {
            chain_id: entry.chain_id,
            registry: entry.registry,
            context_id: entry.deployment_context_id,
        };
        if bound_to == to {
            plan.skipped.push(i);
        } else if bound_to == from {
            plan.to_migrate.push(i);
        } else {
            bail!(
                "lockfile entry {} at {:?} is bound to {}:{} (context {}) — neither the \
                 --from profile's context {} nor the active profile's {}; a migration carries \
                 entries from exactly one previous profile, all or none",
                entry.name,
                entry.install_dir,
                entry.chain_id,
                entry.registry,
                entry.deployment_context_id,
                from.context_id,
                to.context_id
            );
        }
    }
    Ok(plan)
}

/// Apply the plan to the lockfile given the fresh checks: exactly one check
/// per planned entry, all transitions run on a copy, and the lockfile is
/// replaced only when every one succeeded — the caller then saves it once.
/// On any failure the lockfile is untouched and the error names the entry.
pub fn apply(
    plan: &MigrationPlan,
    lock: &mut Lockfile,
    checks: &[Checked],
    anchor_block: u64,
    anchor_block_hash: B256,
    now: u64,
) -> Result<Summary> {
    let mut wanted: Vec<usize> = plan.to_migrate.clone();
    wanted.sort_unstable();
    let mut given: Vec<usize> = checks.iter().map(|c| c.index).collect();
    given.sort_unstable();
    if wanted != given {
        bail!(
            "migration checks do not cover exactly the planned entries (planned {:?}, \
             checked {:?}) — refusing to rebind anything",
            wanted,
            given
        );
    }
    let mut entries = lock.entries.clone();
    let mut summary = Summary {
        skipped: plan.skipped.len(),
        ..Summary::default()
    };
    for check in checks {
        let entry = entries
            .get_mut(check.index)
            .ok_or_else(|| eyre::eyre!("migration check names a missing entry {}", check.index))?;
        let name = entry.name.clone();
        let state = migrate_transition(
            entry,
            &plan.from,
            &plan.to,
            check.fresh_status,
            &check.local,
            anchor_block,
            anchor_block_hash,
            now,
        )
        .map_err(|e| eyre::eyre!("migrating entry {name}: {e}"))?;
        summary.migrated += 1;
        match state {
            "current" => summary.current += 1,
            "reenable-required" => summary.reenable_required += 1,
            "quarantined" => summary.quarantined += 1,
            "revoked" => summary.revoked += 1,
            "blocked" => summary.blocked += 1,
            "modified" => summary.modified += 1,
            "incomplete" => summary.incomplete += 1,
            other => bail!("migrating entry {name}: unknown state {other:?}"),
        }
        summary.entries.push(EntryResult {
            name,
            install_dir: entry.install_dir.clone(),
            fresh_status: check.fresh_status,
            state: state.into(),
            local_integrity: check.local.label(),
            sticky_suspension: entry.sticky_suspension.clone(),
        });
    }
    lock.entries = entries;
    Ok(summary)
}

/// A migration aborted after some entries were already proven under the new
/// profile: persist every ADVERSE observation among them (quarantined or
/// revoked) as a sticky suspension plus an observation record, without
/// rebinding any entry — the caller saves the lockfile and reports. Entries
/// that were not proven, or proved a non-adverse status, are left exactly as
/// they were: an unfinished migration never fabricates a status. Returns the
/// affected entries for the report.
pub fn record_partial_failure(
    lock: &mut Lockfile,
    plan: &MigrationPlan,
    observed: &[Observed],
    anchor_block: u64,
    anchor_block_hash: B256,
    now: u64,
) -> Vec<Affected> {
    let mut affected = Vec::new();
    for o in observed {
        if !plan.to_migrate.contains(&o.index) {
            continue;
        }
        let Some(entry) = lock.entries.get_mut(o.index) else {
            continue;
        };
        if entry.deployment_context_id != plan.from.context_id {
            continue;
        }
        if let Some(sticky) = record_observation(
            entry,
            &plan.to,
            o.fresh_status,
            anchor_block,
            anchor_block_hash,
            now,
        ) {
            affected.push(Affected {
                name: entry.name.clone(),
                install_dir: entry.install_dir.clone(),
                fresh_status: o.fresh_status,
                sticky_suspension: sticky,
            });
        }
    }
    affected
}

/// How a failed save of the migrated lockfile is reported: only a failure
/// BEFORE publication leaves the previous lockfile in place; after it, the
/// new contents are visible with unconfirmed durability, and the recovery is
/// a locked re-read plus a re-run, which republishes the unchanged lockfile
/// durably once every entry is at the new context.
pub fn describe_save_failure(failure: &AtomicWriteFailure) -> String {
    match failure.phase {
        WritePhase::BeforePublish => format!(
            "the migrated lockfile could not be written ({:#}); the previous lockfile is \
             intact and no entry was rebound",
            failure.error
        ),
        WritePhase::AfterPublish => format!(
            "the migrated lockfile was published but its durability could not be confirmed \
             ({:#}); re-read it under the lock before relying on it, then re-run `intend \
             migrate` with the same profiles: entries already at the new context are skipped \
             and the unchanged lockfile is republished durably, so the re-run completes the \
             recovery and reports success only once that sync succeeds",
            failure.error
        ),
    }
}

/// The persistence step, injectable so the orchestration is testable with
/// deterministic write failures. The command passes `Lockfile::save_phased`.
pub type Saver<'a> = &'a dyn Fn(&Lockfile) -> std::result::Result<(), AtomicWriteFailure>;

/// What happened to the safety records of an aborted migration on disk.
#[derive(Debug)]
pub enum SafetySave {
    /// No adverse observation preceded the failure; nothing needed saving.
    NothingToSave,
    /// The records reached the lockfile and the write was confirmed durable.
    Durable,
    /// The records are visible in the lockfile but the write's durability
    /// could not be confirmed.
    Published(AtomicWriteFailure),
    /// The write failed before publication: the records exist in memory only
    /// and the lockfile on disk does not carry them.
    NotSaved(AtomicWriteFailure),
}

/// The report of an aborted migration, truthful about what reached the disk:
/// the failing entry, then the affected entries with the proof context and
/// anchor their observations were made under, then the fate of the safety
/// save. Names, context and anchor appear in every variant so an operator
/// knows which restrictions are and are not guaranteed to survive.
#[allow(clippy::too_many_arguments)]
pub fn describe_abort(
    failing_name: &str,
    failing_dir: &str,
    error: &str,
    plan: &MigrationPlan,
    anchor_block: u64,
    anchor_block_hash: B256,
    affected: &[Affected],
    safety: &SafetySave,
) -> String {
    let head = format!(
        "fresh check of entry {failing_name} at {failing_dir:?} failed at block {anchor_block}: \
         {error}. No entry was rebound."
    );
    if affected.is_empty() {
        return format!(
            "{head} No adverse status had been proven before the failure; nothing else was \
             recorded."
        );
    }
    let list: Vec<String> = affected
        .iter()
        .map(|a| {
            format!(
                "{} at {:?} (status {}, sticky suspension {:?})",
                a.name, a.install_dir, a.fresh_status, a.sticky_suspension
            )
        })
        .collect();
    let what = format!(
        "Adverse observations proven under the new profile (context {}, anchor block \
         {anchor_block}, hash {anchor_block_hash}) for {} require an explicit `intend enable` \
         after the migration completes; the entries stay bound to their previous context {}.",
        plan.to.context_id,
        list.join("; "),
        plan.from.context_id
    );
    let fate = match safety {
        SafetySave::NothingToSave => String::new(),
        SafetySave::Durable => " They were recorded in the lockfile as sticky suspensions and \
                               observation records, and the write was confirmed durable."
            .into(),
        SafetySave::Published(f) => format!(
            " They were written to the lockfile, but the write's durability could not be \
             confirmed ({:#}); re-read the lockfile under the lock and re-run the migration \
             once the cause is fixed.",
            f.error
        ),
        SafetySave::NotSaved(f) => format!(
            " Saving them FAILED before publication ({:#}): the lockfile on disk does NOT carry \
             these restrictions, they existed in this process only. Treat the named entries \
             as suspended until a re-run records them.",
            f.error
        ),
    };
    format!("{head} {what}{fate}")
}

/// The fresh evidence gathered for one planned entry, in plan order: the
/// local verdict, and the point check under the new profile or the error
/// that stopped the sweep. The command stops gathering at the first error,
/// so evidence after it is absent by construction.
#[derive(Debug, Clone)]
pub struct Evidence {
    pub index: usize,
    pub local: LocalVerdict,
    pub fresh: std::result::Result<u8, String>,
}

/// The result of a migration run.
#[derive(Debug)]
pub enum Outcome {
    /// Every planned entry was rebound and the lockfile saved durably.
    Migrated(Summary),
}

/// The migration orchestration, shared by the command and the tests: walk the
/// evidence in plan order; at the first point-check error persist the
/// adverse observations already proven (a safety-only save, no rebinding)
/// and fail with a report truthful about that save; otherwise apply every
/// transition and save once, failing with the save's phase if it fails.
pub fn run(
    lock: &mut Lockfile,
    plan: &MigrationPlan,
    evidence: &[Evidence],
    anchor_block: u64,
    anchor_block_hash: B256,
    now: u64,
    save: Saver,
) -> Result<Outcome> {
    let mut checks = Vec::with_capacity(evidence.len());
    let mut observed = Vec::with_capacity(evidence.len());
    for ev in evidence {
        match &ev.fresh {
            Ok(status) => {
                observed.push(Observed {
                    index: ev.index,
                    fresh_status: *status,
                });
                checks.push(Checked {
                    index: ev.index,
                    fresh_status: *status,
                    local: ev.local.clone(),
                });
            }
            Err(error) => {
                let (name, dir) = match lock.entries.get(ev.index) {
                    Some(e) => (e.name.clone(), e.install_dir.clone()),
                    None => (format!("#{}", ev.index), String::new()),
                };
                let affected = record_partial_failure(
                    lock,
                    plan,
                    &observed,
                    anchor_block,
                    anchor_block_hash,
                    now,
                );
                let safety = if affected.is_empty() {
                    SafetySave::NothingToSave
                } else {
                    match save(lock) {
                        Ok(()) => SafetySave::Durable,
                        Err(f) if f.phase == WritePhase::AfterPublish => SafetySave::Published(f),
                        Err(f) => SafetySave::NotSaved(f),
                    }
                };
                bail!(
                    "{}",
                    describe_abort(
                        &name,
                        &dir,
                        error,
                        plan,
                        anchor_block,
                        anchor_block_hash,
                        &affected,
                        &safety
                    )
                );
            }
        }
    }
    let summary = apply(plan, lock, &checks, anchor_block, anchor_block_hash, now)?;
    if let Err(f) = save(lock) {
        bail!("{}", describe_save_failure(&f));
    }
    Ok(Outcome::Migrated(summary))
}

/// The all-skipped case: every entry is already at the new context, which is
/// also what a re-run after an unconfirmed save sees. Reporting success then
/// requires confirming durability, so the unchanged loaded contents are
/// republished through the saver — no migration record appended, no
/// restriction changed — and a failure stays a failure. An empty lockfile has
/// nothing to confirm.
pub fn confirm_durable(lock: &Lockfile, save: Saver) -> Result<bool> {
    if lock.entries.is_empty() {
        return Ok(false);
    }
    match save(lock) {
        Ok(()) => Ok(true),
        Err(f) => bail!(
            "every entry is already at the new context, but republishing the lockfile to \
             confirm its durability failed: {}",
            describe_save_failure(&f)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::describe_save_failure;
    use crate::store::{AtomicWriteFailure, WritePhase};

    #[test]
    fn save_failures_only_promise_an_intact_lockfile_before_publication() {
        let before = describe_save_failure(&AtomicWriteFailure {
            phase: WritePhase::BeforePublish,
            error: eyre::eyre!("disk full"),
        });
        assert!(before.contains("previous lockfile is intact"), "{before}");
        let after = describe_save_failure(&AtomicWriteFailure {
            phase: WritePhase::AfterPublish,
            error: eyre::eyre!("fsync failed"),
        });
        assert!(
            after.contains("durability could not be confirmed"),
            "{after}"
        );
        assert!(after.contains("republished durably"), "{after}");
        assert!(!after.contains("intact"), "{after}");
    }
}
