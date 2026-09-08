//! Policy-version migration of a lockfile (`intend migrate`; spec §3
//! `policyVersions`). A profile that accepts a newly announced policy version
//! has a different deployment context id, so every entry installed under the
//! previous profile is refused by `audit` and `enable` until it is carried
//! across explicitly: the new profile must be a strict SUCCESSOR of the old
//! one (`Profile::is_successor_of`), every entry must be bound to the old
//! context (or already to the new one, in which case it is skipped), and every
//! migrated entry gets a fresh point check UNDER THE NEW PROFILE plus a local
//! integrity pass before it is rebound. All entries move or none do.
//!
//! The network-free parts live here so they are testable with fixtures: a
//! plan (which entries move), and its application given the fresh checks the
//! caller obtained. `main.rs` supplies the anchor and the point checks.

use alloy::primitives::B256;
use eyre::{bail, Result};

use crate::lockfile::{migrate_transition, Lockfile, ProofContext};
use crate::profile::Profile;

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

/// The fresh evidence the caller obtained for one entry under the NEW profile.
#[derive(Debug, Clone, Copy)]
pub struct Checked {
    pub index: usize,
    pub fresh_status: u8,
    pub local_intact: bool,
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
}

impl Summary {
    /// Entries that carry a sticky suspension after the migration.
    pub fn suspended(&self) -> usize {
        self.quarantined + self.reenable_required
    }
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
            check.local_intact,
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
    }
    lock.entries = entries;
    Ok(summary)
}
