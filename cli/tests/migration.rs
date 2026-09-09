//! Policy-version migration suite (`intend migrate`): the successor relation
//! gates it, entries move all-or-nothing, sticky suspensions and pending
//! journals are preserved, fresh states are recorded, adverse observations
//! survive an aborted run, quarantined or unpublished installs migrate
//! without activation, and the lockfile keeps every record. The network-free
//! plan/apply/partial-failure functions are exercised with fixture profiles,
//! hand-built entries and real installed trees; the CLI's own migrate command
//! feeds them the anchor and per-entry point checks.

use alloy::primitives::{Address, Bytes, B256};
use intend::lockfile::{
    audit_state_for, enable_transition, AuditRecord, Entry, LocalVerdict, LockedFile, Lockfile,
    ProofContext,
};
use intend::migrate::{apply, local_verdict, prepare, record_partial_failure, Checked, Observed};
use intend::profile::{PolicyUpdate, Profile};

const CID: &str = "bafybeidgtfsc2ro3pfmyggmbz4ea7xg7g4gpehqur7klaadtreyjz6s3fu";

fn profile_a() -> Profile {
    Profile {
        chain_id: 100,
        genesis_hash: B256::repeat_byte(0x01),
        registry: Address::repeat_byte(0x02),
        registry_code_hash: B256::repeat_byte(0x03),
        arbitrator: "0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002"
            .parse()
            .unwrap(),
        arbitrator_extra_data: Bytes::from(vec![0u8; 64]),
        governor: Address::repeat_byte(0x04),
        policy_updates: Vec::new(),
        anchor_rpcs: vec!["http://127.0.0.1:1".into(), "http://127.0.0.1:2".into()],
        anchor_operators: Vec::new(),
        snapshot_urls: Vec::new(),
        registration_meta_evidence: format!("/ipfs/{CID}/registration.json"),
        clearing_meta_evidence: format!("/ipfs/{CID}/clearing.json"),
        provider_rpc: None,
        gateways: Vec::new(),
        test_headerless_state_root: false,
    }
}

fn update(n: u64) -> PolicyUpdate {
    PolicyUpdate {
        updates: n,
        registration_meta_evidence: format!("/ipfs/{CID}/registration-{n}.json"),
        clearing_meta_evidence: format!("/ipfs/{CID}/clearing-{n}.json"),
    }
}

/// A + policy version 1: the profile a 2.3 activation ships.
fn profile_b() -> Profile {
    let mut p = profile_a();
    p.policy_updates.push(update(1));
    p
}

/// B + policy version 2: a later transition.
fn profile_d() -> Profile {
    let mut p = profile_b();
    p.policy_updates.push(update(2));
    p
}

/// A + version 1 but under ANOTHER governor: a different trust decision.
fn profile_c() -> Profile {
    let mut p = profile_b();
    p.governor = Address::repeat_byte(0x99);
    p
}

fn entry(name: &str, dir: &str, ctx: &ProofContext) -> Entry {
    Entry {
        name: name.into(),
        item_id: alloy::primitives::keccak256(name.as_bytes()),
        tree_cid: CID.into(),
        chain_id: ctx.chain_id,
        registry: ctx.registry,
        deployment_context_id: ctx.context_id,
        install_dir: dir.into(),
        installed_at_unix: 1,
        status_at_install: 1,
        anchor_block: 7,
        anchor_block_hash: B256::repeat_byte(0x07),
        anchor_state_root: B256::repeat_byte(0x08),
        root_block_sha256: String::new(),
        total_bytes: 0,
        blocks: 0,
        files: Vec::new(),
        dirs: Vec::new(),
        audit: None,
        sticky_suspension: None,
        pending: false,
        migrations: Vec::new(),
        observations: Vec::new(),
    }
}

/// A real installed tree on disk with a manifest that verifies: SKILL.md and
/// one more file, digests computed the way the installer records them.
fn installed(base: &std::path::Path, name: &str, ctx: &ProofContext) -> Entry {
    use sha2::{Digest, Sha256};
    let dir = base.join(name);
    std::fs::create_dir(&dir).unwrap();
    let files = [
        ("SKILL.md", format!("---\nname: {name}\n---\nbody\n")),
        ("a.md", "alpha".into()),
    ];
    let mut locked = Vec::new();
    for (path, content) in &files {
        std::fs::write(dir.join(path), content.as_bytes()).unwrap();
        let digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        locked.push(LockedFile {
            path: (*path).into(),
            bytes: content.len() as u64,
            sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        });
    }
    let mut e = entry(name, dir.to_str().unwrap(), ctx);
    e.files = locked;
    e
}

fn lock_with(entries: Vec<Entry>) -> Lockfile {
    Lockfile {
        version: intend::lockfile::LOCKFILE_VERSION,
        entries,
    }
}

fn intact(index: usize, fresh_status: u8) -> Checked {
    Checked {
        index,
        fresh_status,
        local: LocalVerdict::Intact,
    }
}

fn registered(plan: &intend::migrate::MigrationPlan) -> Vec<Checked> {
    plan.to_migrate.iter().map(|&i| intact(i, 1)).collect()
}

#[test]
fn migrate_rebinds_entries_and_only_the_new_profile_audits_them() {
    let (a, b) = (profile_a(), profile_b());
    let mut lock = lock_with(vec![
        entry("one", "/skills/one", &a.proof_context()),
        entry("two", "/skills/two", &a.proof_context()),
    ]);
    // Installed under A: A audits them, B refuses them and names the migration.
    lock.assert_entries_bound(&a.proof_context()).unwrap();
    let err = format!(
        "{:#}",
        lock.assert_entries_bound(&b.proof_context()).unwrap_err()
    );
    assert!(err.contains("intend migrate --from"), "{err}");
    assert!(err.contains("release asset"), "{err}");

    let plan = prepare(&b, &a, &lock).unwrap();
    assert_eq!(plan.to_migrate, vec![0, 1]);
    assert!(plan.skipped.is_empty());
    let summary = apply(
        &plan,
        &mut lock,
        &registered(&plan),
        1234,
        B256::repeat_byte(0xaa),
        99,
    )
    .unwrap();
    assert_eq!(
        (summary.migrated, summary.skipped, summary.current),
        (2, 0, 2)
    );
    assert_eq!(summary.suspended(), 0);
    assert_eq!(summary.entries.len(), 2);

    // Now B audits them and A no longer does.
    lock.assert_entries_bound(&b.proof_context()).unwrap();
    let err = format!(
        "{:#}",
        lock.assert_entries_bound(&a.proof_context()).unwrap_err()
    );
    assert!(err.contains("intend migrate --from"), "{err}");
    for e in &lock.entries {
        assert_eq!(e.deployment_context_id, b.deployment_context_id());
        assert_eq!(e.migrations.len(), 1);
        let m = &e.migrations[0];
        assert_eq!(m.from_context, a.deployment_context_id());
        assert_eq!(m.to_context, b.deployment_context_id());
        assert_eq!(
            (
                m.anchor_block,
                m.status,
                m.state.as_str(),
                m.local_integrity.as_str()
            ),
            (1234, 1, "current", "intact")
        );
        assert!(
            m.previous_audit.is_none(),
            "there was no audit record before"
        );
        assert_eq!(e.audit.as_ref().unwrap().state, "current");
        assert!(e.sticky_suspension.is_none());
    }
}

#[test]
fn migrate_refuses_a_profile_that_is_not_a_successor_and_leaves_the_lockfile_alone() {
    let a = profile_a();
    let lock = lock_with(vec![entry("one", "/skills/one", &a.proof_context())]);
    let before = serde_json::to_string(&lock).unwrap();
    // C changes the governor: a different trust decision, never a successor.
    let err = format!("{:#}", prepare(&profile_c(), &a, &lock).unwrap_err());
    assert!(err.contains("not a successor"), "{err}");
    assert!(err.contains("governor"), "{err}");
    // Identical profiles: nothing to migrate.
    let err = format!("{:#}", prepare(&a, &a, &lock).unwrap_err());
    assert!(err.contains("identical"), "{err}");
    // Going backwards (B → A) is refused too.
    let err = format!("{:#}", prepare(&a, &profile_b(), &lock).unwrap_err());
    assert!(err.contains("only ADDS"), "{err}");
    assert_eq!(serde_json::to_string(&lock).unwrap(), before);
}

#[test]
fn migrate_preserves_sticky_suspensions_and_records_fresh_states() {
    let (a, b) = (profile_a(), profile_b());
    let mut revoked_before = entry("revoked-before", "/skills/r", &a.proof_context());
    revoked_before.sticky_suspension = Some("revoked".into());
    let mut pending = entry("pending", "/skills/p", &a.proof_context());
    pending.pending = true;
    let mut lock = lock_with(vec![
        revoked_before,
        entry("now-clearing", "/skills/c", &a.proof_context()),
        entry("now-absent", "/skills/x", &a.proof_context()),
        pending,
        entry("fine", "/skills/f", &a.proof_context()),
    ]);
    let plan = prepare(&b, &a, &lock).unwrap();
    let checks = [
        intact(0, 1),
        intact(1, 3),
        intact(2, 0),
        intact(3, 1),
        intact(4, 1),
    ];
    let summary = apply(
        &plan,
        &mut lock,
        &checks,
        5000,
        B256::repeat_byte(0xbb),
        100,
    )
    .unwrap();
    assert_eq!(summary.migrated, 5);
    assert_eq!(
        (
            summary.current,
            summary.reenable_required,
            summary.quarantined,
            summary.revoked,
            summary.incomplete
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(summary.suspended(), 3);
    let by_name = |n: &str| lock.entries.iter().find(|e| e.name == n).unwrap().clone();
    // A sticky suspension survives a fresh Registered proof: only `enable` clears it.
    let e = by_name("revoked-before");
    assert_eq!(e.sticky_suspension.as_deref(), Some("revoked"));
    assert_eq!(e.audit.as_ref().unwrap().state, "reenable-required");
    // A quarantine or revocation observed at migration is acquired and recorded.
    let e = by_name("now-clearing");
    assert_eq!(e.sticky_suspension.as_deref(), Some("quarantined"));
    assert_eq!(e.migrations[0].state, "quarantined");
    let e = by_name("now-absent");
    assert_eq!(e.sticky_suspension.as_deref(), Some("revoked"));
    assert_eq!(e.migrations[0].status, 0);
    // A pending journal stays pending and is reported incomplete, not dropped.
    let e = by_name("pending");
    assert!(e.pending);
    assert_eq!(e.audit.as_ref().unwrap().state, "incomplete");
    // Everything moved to B, nothing was left behind.
    assert!(lock
        .entries
        .iter()
        .all(|e| e.deployment_context_id == b.deployment_context_id()));
    lock.assert_entries_bound(&b.proof_context()).unwrap();
}

#[test]
fn migrate_is_all_or_nothing_and_skips_entries_already_migrated() {
    let (a, b) = (profile_a(), profile_b());
    // Mixed contexts: an entry from an unrelated context aborts the plan.
    let stranger = ProofContext {
        chain_id: 100,
        registry: Address::repeat_byte(0x02),
        context_id: B256::repeat_byte(0xee),
    };
    let lock = lock_with(vec![
        entry("one", "/skills/one", &a.proof_context()),
        entry("odd", "/skills/odd", &stranger),
    ]);
    let err = format!("{:#}", prepare(&b, &a, &lock).unwrap_err());
    assert!(err.contains("odd") && err.contains("all or none"), "{err}");

    // Half-migrated lockfile: the B-bound entry is skipped, the A-bound one moves.
    let mut lock = lock_with(vec![
        entry("done", "/skills/done", &b.proof_context()),
        entry("todo", "/skills/todo", &a.proof_context()),
    ]);
    let plan = prepare(&b, &a, &lock).unwrap();
    assert_eq!(
        (plan.skipped.clone(), plan.to_migrate.clone()),
        (vec![0], vec![1])
    );
    // Checks that do not cover the plan exactly leave the lockfile untouched.
    let before = serde_json::to_string(&lock).unwrap();
    let err = format!(
        "{:#}",
        apply(&plan, &mut lock, &[], 1, B256::ZERO, 1).unwrap_err()
    );
    assert!(err.contains("do not cover"), "{err}");
    let wrong = [intact(0, 1)];
    assert!(apply(&plan, &mut lock, &wrong, 1, B256::ZERO, 1).is_err());
    assert_eq!(serde_json::to_string(&lock).unwrap(), before);
    // The right check migrates exactly the one entry.
    let summary = apply(&plan, &mut lock, &registered(&plan), 1, B256::ZERO, 1).unwrap();
    assert_eq!((summary.migrated, summary.skipped), (1, 1));
    assert!(lock.entries[0].migrations.is_empty());
    assert_eq!(lock.entries[1].migrations.len(), 1);
    lock.assert_entries_bound(&b.proof_context()).unwrap();
}

#[test]
fn chained_migrations_accumulate_records_and_round_trip_through_the_lockfile() {
    let (a, b, d) = (profile_a(), profile_b(), profile_d());
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("intend-lock.json");
    let mut lock = lock_with(vec![entry("one", "/skills/one", &a.proof_context())]);
    let plan = prepare(&b, &a, &lock).unwrap();
    apply(
        &plan,
        &mut lock,
        &registered(&plan),
        10,
        B256::repeat_byte(0x10),
        10,
    )
    .unwrap();
    lock.save(&path).unwrap();
    // A→B on disk; B→D next, from the reloaded file.
    let mut lock = Lockfile::load(&path).unwrap();
    let plan = prepare(&d, &b, &lock).unwrap();
    apply(
        &plan,
        &mut lock,
        &registered(&plan),
        20,
        B256::repeat_byte(0x20),
        20,
    )
    .unwrap();
    lock.save(&path).unwrap();
    let lock = Lockfile::load(&path).unwrap();
    let e = &lock.entries[0];
    assert_eq!(e.deployment_context_id, d.deployment_context_id());
    assert_eq!(e.migrations.len(), 2);
    assert_eq!(e.migrations[0].to_context, b.deployment_context_id());
    assert_eq!(e.migrations[1].from_context, b.deployment_context_id());
    assert_eq!(e.migrations[1].to_context, d.deployment_context_id());
    assert_eq!(
        (e.migrations[0].anchor_block, e.migrations[1].anchor_block),
        (10, 20)
    );
    // The second migration keeps the first one's audit record.
    assert_eq!(
        e.migrations[1]
            .previous_audit
            .as_ref()
            .unwrap()
            .anchor_block,
        10
    );
    // A→D directly is also a successor step (strict prefix), for a consumer
    // that skipped a release.
    let mut fresh = lock_with(vec![entry("one", "/skills/one", &a.proof_context())]);
    let plan = prepare(&d, &a, &fresh).unwrap();
    apply(&plan, &mut fresh, &registered(&plan), 30, B256::ZERO, 30).unwrap();
    assert_eq!(
        fresh.entries[0].deployment_context_id,
        d.deployment_context_id()
    );
}

#[test]
fn lockfiles_without_migration_records_still_load() {
    // A v2 lockfile written before this feature has neither `migrations`
    // nor `observations`.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("intend-lock.json");
    let a = profile_a();
    let mut lock = lock_with(vec![entry("one", "/skills/one", &a.proof_context())]);
    lock.save(&path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("migrations") && !raw.contains("observations"),
        "empty records are not serialized"
    );
    let loaded = Lockfile::load(&path).unwrap();
    assert!(loaded.entries[0].migrations.is_empty());
    // And a record, once written, comes back exactly.
    let plan = prepare(&profile_b(), &a, &lock).unwrap();
    apply(
        &plan,
        &mut lock,
        &registered(&plan),
        1,
        B256::repeat_byte(0x01),
        2,
    )
    .unwrap();
    lock.save(&path).unwrap();
    let loaded = Lockfile::load(&path).unwrap();
    assert_eq!(loaded.entries[0].migrations[0].migrated_at_unix, 2);
    assert_eq!(
        loaded.entries[0].migrations[0].anchor_block_hash,
        B256::repeat_byte(0x01)
    );
}

#[test]
fn aborted_migration_persists_adverse_observations_and_the_retry_keeps_the_restriction() {
    let (a, b) = (profile_a(), profile_b());
    for adverse in [3u8, 0u8] {
        let mut lock = lock_with(vec![
            entry("first", "/skills/first", &a.proof_context()),
            entry("failing", "/skills/failing", &a.proof_context()),
            entry("fine", "/skills/fine", &a.proof_context()),
        ]);
        let plan = prepare(&b, &a, &lock).unwrap();
        // "first" proved adverse, then "failing"'s point check failed: nothing
        // is rebound, but the adverse observation is persisted as a sticky
        // suspension with its new-profile context, and the report names it.
        let observed = [Observed {
            index: 0,
            fresh_status: adverse,
        }];
        let affected =
            record_partial_failure(&mut lock, &plan, &observed, 500, B256::repeat_byte(0x50), 7);
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0].name, "first");
        let expected_sticky = if adverse == 3 {
            "quarantined"
        } else {
            "revoked"
        };
        assert_eq!(affected[0].sticky_suspension, expected_sticky);
        let first = &lock.entries[0];
        assert_eq!(
            first.deployment_context_id,
            a.deployment_context_id(),
            "not rebound"
        );
        assert_eq!(first.sticky_suspension.as_deref(), Some(expected_sticky));
        assert_eq!(first.observations.len(), 1);
        assert_eq!(first.observations[0].context, b.deployment_context_id());
        assert_eq!(first.observations[0].status, adverse);
        assert_eq!(first.observations[0].reason, "migration-aborted");
        assert!(first.audit.is_none(), "no audit record is fabricated");
        // Unproven entries are untouched.
        assert!(
            lock.entries[1].sticky_suspension.is_none()
                && lock.entries[2].sticky_suspension.is_none()
        );
        assert!(lock.entries[1].observations.is_empty());
        // The old profile still audits the lockfile, and its sweep now sees
        // the restriction: a fresh Registered proof is reenable-required.
        lock.assert_entries_bound(&a.proof_context()).unwrap();
        assert_eq!(
            audit_state_for(&lock.entries[0], 1, true).0,
            "reenable-required"
        );
        // Retry: "first" is Registered again. The migration completes with
        // the restriction in force; only an explicit enable clears it.
        let plan = prepare(&b, &a, &lock).unwrap();
        let summary = apply(
            &plan,
            &mut lock,
            &registered(&plan),
            600,
            B256::repeat_byte(0x60),
            8,
        )
        .unwrap();
        assert_eq!(
            (summary.migrated, summary.reenable_required, summary.current),
            (3, 1, 2)
        );
        let first = &lock.entries[0];
        assert_eq!(first.deployment_context_id, b.deployment_context_id());
        assert_eq!(first.sticky_suspension.as_deref(), Some(expected_sticky));
        assert_eq!(first.audit.as_ref().unwrap().state, "reenable-required");
        assert_eq!(
            first.observations.len(),
            1,
            "the observation stays in the history"
        );
        // A non-adverse observation records nothing.
        let mut fresh = lock_with(vec![entry("x", "/skills/x", &a.proof_context())]);
        let plan = prepare(&b, &a, &fresh).unwrap();
        let affected = record_partial_failure(
            &mut fresh,
            &plan,
            &[Observed {
                index: 0,
                fresh_status: 1,
            }],
            1,
            B256::ZERO,
            1,
        );
        assert!(affected.is_empty() && fresh.entries[0].observations.is_empty());
        assert!(fresh.entries[0].sticky_suspension.is_none());
    }
}

#[test]
fn quarantined_moved_and_unpublished_installs_migrate_without_activation() {
    let (a, b) = (profile_a(), profile_b());
    let tmp = tempfile::tempdir().unwrap();
    let intact_entry = installed(tmp.path(), "intact", &a.proof_context());
    let moved = installed(tmp.path(), "moved", &a.proof_context());
    // The bootstrap's quarantine procedure: the tree leaves the discovery
    // path, the lockfile entry stays.
    std::fs::rename(
        tmp.path().join("moved"),
        tmp.path().join("quarantine-moved"),
    )
    .unwrap();
    let mut never_published = entry(
        "pending",
        tmp.path().join("pending").to_str().unwrap(),
        &a.proof_context(),
    );
    never_published.pending = true;
    let mut lock = lock_with(vec![intact_entry, moved, never_published]);
    let plan = prepare(&b, &a, &lock).unwrap();
    // The real verdicts from the real filesystem.
    let verdicts: Vec<LocalVerdict> = lock.entries.iter().map(local_verdict).collect();
    assert!(verdicts[0].is_intact());
    assert!(
        verdicts[1].label().contains("missing"),
        "{}",
        verdicts[1].label()
    );
    assert!(!verdicts[2].is_intact());
    // The moved tree's item is ClearingRequested: the integrity failure of
    // the same entry, found later, does not lose that observation.
    let checks = vec![
        Checked {
            index: 0,
            fresh_status: 1,
            local: verdicts[0].clone(),
        },
        Checked {
            index: 1,
            fresh_status: 3,
            local: verdicts[1].clone(),
        },
        Checked {
            index: 2,
            fresh_status: 1,
            local: verdicts[2].clone(),
        },
    ];
    let summary = apply(&plan, &mut lock, &checks, 900, B256::repeat_byte(0x90), 9).unwrap();
    assert_eq!(
        (
            summary.migrated,
            summary.current,
            summary.modified,
            summary.incomplete
        ),
        (3, 1, 1, 1)
    );
    assert_eq!(summary.suspended(), 1);
    // Every entry moved to the new context; none had its bytes reported
    // intact except the intact one.
    for e in &lock.entries {
        assert_eq!(e.deployment_context_id, b.deployment_context_id());
    }
    let by_name = |n: &str| lock.entries.iter().find(|e| e.name == n).unwrap().clone();
    let m = by_name("moved");
    assert_eq!(m.audit.as_ref().unwrap().state, "modified");
    assert_eq!(m.sticky_suspension.as_deref(), Some("quarantined"));
    assert!(m.migrations[0].local_integrity.contains("missing"));
    assert_eq!(m.migrations[0].state, "modified");
    assert_eq!(m.files.len(), 2, "the manifest is preserved");
    let pnd = by_name("pending");
    assert!(pnd.pending);
    assert_eq!(pnd.audit.as_ref().unwrap().state, "incomplete");
    assert_eq!(by_name("intact").migrations[0].local_integrity, "intact");
    assert!(summary
        .entries
        .iter()
        .find(|e| e.name == "moved")
        .unwrap()
        .local_integrity
        .contains("missing"));
    // The moved tree cannot be enabled where it is not: enable verifies at
    // the recorded path, and a failed enable leaves the restriction.
    let mut m = m;
    let err = format!(
        "{:#}",
        enable_transition(&mut m, &b.proof_context(), 1).unwrap_err()
    );
    assert!(err.contains("missing"), "{err}");
    assert_eq!(
        m.sticky_suspension.as_deref(),
        Some("quarantined"),
        "untouched on failure"
    );
    // Audit under the new profile accepts the lockfile as a whole.
    lock.assert_entries_bound(&b.proof_context()).unwrap();
}

#[test]
fn the_previous_audit_record_is_kept_in_the_migration_record() {
    let (a, b) = (profile_a(), profile_b());
    let mut e = entry("audited", "/skills/audited", &a.proof_context());
    e.audit = Some(AuditRecord {
        checked_at_unix: 42,
        anchor_block: 4200,
        anchor_block_hash: B256::repeat_byte(0x42),
        status: 1,
        state: "current".into(),
    });
    let mut lock = lock_with(vec![e]);
    let plan = prepare(&b, &a, &lock).unwrap();
    apply(
        &plan,
        &mut lock,
        &registered(&plan),
        5000,
        B256::repeat_byte(0x50),
        50,
    )
    .unwrap();
    let e = &lock.entries[0];
    let prev = e.migrations[0]
        .previous_audit
        .as_ref()
        .expect("the old record is kept");
    assert_eq!(
        (prev.checked_at_unix, prev.anchor_block, prev.status),
        (42, 4200, 1)
    );
    assert_eq!(e.migrations[0].from_context, a.deployment_context_id());
    assert_eq!(
        e.audit.as_ref().unwrap().checked_at_unix,
        50,
        "the current record is the fresh one"
    );
    // Round trip through the file keeps it.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("intend-lock.json");
    lock.save(&path).unwrap();
    let loaded = Lockfile::load(&path).unwrap();
    assert_eq!(
        loaded.entries[0].migrations[0]
            .previous_audit
            .as_ref()
            .unwrap()
            .anchor_block,
        4200
    );
}

// ---- the production orchestration with injected save failures ----

use intend::migrate::{confirm_durable, run, Evidence, Outcome};
use intend::store::{atomic_write_with_sync, AtomicWriteFailure, WritePhase};
use std::cell::Cell;

fn evidence(index: usize, fresh: Result<u8, &str>) -> Evidence {
    Evidence {
        index,
        local: LocalVerdict::Intact,
        fresh: fresh.map_err(str::to_owned),
    }
}

/// A saver that really writes through the atomic path but whose parent
/// directory sync fails: the new contents are published, durability is not.
fn saver_after_publish(
    path: &std::path::Path,
) -> impl Fn(&Lockfile) -> Result<(), AtomicWriteFailure> + '_ {
    move |lock: &Lockfile| {
        let bytes = serde_json::to_vec_pretty(lock).unwrap();
        atomic_write_with_sync(path, &bytes, &|_| {
            Err(std::io::Error::other("simulated fsync failure"))
        })
    }
}

#[test]
fn a_rerun_after_an_unconfirmed_save_republishes_the_lockfile_before_reporting_success() {
    let (a, b) = (profile_a(), profile_b());
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("intend-lock.json");
    let mut lock = lock_with(vec![
        entry("one", "/skills/one", &a.proof_context()),
        entry("two", "/skills/two", &a.proof_context()),
    ]);
    lock.save(&path).unwrap();
    // The migration itself: the rename publishes, the directory sync fails.
    let plan = prepare(&b, &a, &lock).unwrap();
    let ev = vec![evidence(0, Ok(1)), evidence(1, Ok(1))];
    let err = format!(
        "{:#}",
        run(
            &mut lock,
            &plan,
            &ev,
            100,
            B256::repeat_byte(0x10),
            5,
            &saver_after_publish(&path)
        )
        .unwrap_err()
    );
    assert!(err.contains("durability could not be confirmed"), "{err}");
    assert!(err.contains("republished durably"), "{err}");
    // The new contents are on disk: reloaded, every entry is at the new context.
    let reloaded = Lockfile::load(&path).unwrap();
    assert!(reloaded
        .entries
        .iter()
        .all(|e| e.deployment_context_id == b.deployment_context_id()));
    assert!(reloaded.entries.iter().all(|e| e.migrations.len() == 1));
    let plan = prepare(&b, &a, &reloaded).unwrap();
    assert!(plan.to_migrate.is_empty() && plan.skipped.len() == 2);
    // The re-run: the no-op path must attempt the durable republish. A second
    // sync failure is still an error...
    let attempts = Cell::new(0);
    let failing = |l: &Lockfile| {
        attempts.set(attempts.get() + 1);
        saver_after_publish(&path)(l)
    };
    let err = format!("{:#}", confirm_durable(&reloaded, &failing).unwrap_err());
    assert_eq!(attempts.get(), 1, "a durability attempt happened");
    assert!(err.contains("confirm its durability failed"), "{err}");
    // ...and success comes only when the republish succeeds.
    let attempts = Cell::new(0);
    let real = |l: &Lockfile| {
        attempts.set(attempts.get() + 1);
        l.save_phased(&path)
    };
    assert!(confirm_durable(&reloaded, &real).unwrap());
    assert_eq!(attempts.get(), 1);
    let after = Lockfile::load(&path).unwrap();
    for (x, y) in after.entries.iter().zip(&reloaded.entries) {
        assert_eq!(x.migrations.len(), 1, "no extra migration record");
        assert_eq!(
            x.sticky_suspension, y.sticky_suspension,
            "restrictions unchanged"
        );
        assert_eq!(x.deployment_context_id, y.deployment_context_id);
    }
    // An empty lockfile has nothing to confirm and touches nothing.
    let empty = lock_with(Vec::new());
    let attempts = Cell::new(0);
    let counting = |l: &Lockfile| {
        attempts.set(attempts.get() + 1);
        l.save_phased(&path)
    };
    assert!(!confirm_durable(&empty, &counting).unwrap());
    assert_eq!(attempts.get(), 0);
}

#[test]
fn an_aborted_run_reports_truthfully_what_happened_to_the_safety_records() {
    let (a, b) = (profile_a(), profile_b());
    let make = |tmp: &std::path::Path| {
        let path = tmp.join("intend-lock.json");
        let lock = lock_with(vec![
            entry("first", "/skills/first", &a.proof_context()),
            entry("failing", "/skills/failing", &a.proof_context()),
        ]);
        lock.save(&path).unwrap();
        (path, lock)
    };
    let ev = vec![evidence(0, Ok(3)), evidence(1, Err("rpc down"))];
    let anchor = B256::repeat_byte(0x33);

    // (a) The safety save succeeds: the report says so, the disk carries the
    // restriction, no entry was rebound.
    let tmp = tempfile::tempdir().unwrap();
    let (path, mut lock) = make(tmp.path());
    let plan = prepare(&b, &a, &lock).unwrap();
    let real = |l: &Lockfile| l.save_phased(&path);
    let err = format!(
        "{:#}",
        run(&mut lock, &plan, &ev, 700, anchor, 9, &real).unwrap_err()
    );
    assert!(err.contains("failing") && err.contains("rpc down"), "{err}");
    assert!(
        err.contains("first") && err.contains("quarantined"),
        "{err}"
    );
    assert!(
        err.contains(&format!("{}", b.deployment_context_id())),
        "{err}"
    );
    assert!(err.contains("anchor block 700"), "{err}");
    assert!(err.contains("confirmed durable"), "{err}");
    let disk = Lockfile::load(&path).unwrap();
    assert_eq!(
        disk.entries[0].sticky_suspension.as_deref(),
        Some("quarantined")
    );
    assert_eq!(
        disk.entries[0].deployment_context_id,
        a.deployment_context_id()
    );
    assert!(disk.entries[1].sticky_suspension.is_none());

    // (b) The safety save fails BEFORE publication: the report must not claim
    // persistence, and the disk is unchanged.
    let tmp = tempfile::tempdir().unwrap();
    let (path, mut lock) = make(tmp.path());
    let plan = prepare(&b, &a, &lock).unwrap();
    let before_publish = |_: &Lockfile| -> Result<(), AtomicWriteFailure> {
        Err(AtomicWriteFailure {
            phase: WritePhase::BeforePublish,
            error: eyre::eyre!("disk full"),
        })
    };
    let err = format!(
        "{:#}",
        run(&mut lock, &plan, &ev, 700, anchor, 9, &before_publish).unwrap_err()
    );
    assert!(err.contains("does NOT carry"), "{err}");
    assert!(err.contains("disk full"), "{err}");
    assert!(!err.contains("confirmed durable"), "{err}");
    assert!(
        err.contains("first") && err.contains("anchor block 700"),
        "{err}"
    );
    let disk = Lockfile::load(&path).unwrap();
    assert!(
        disk.entries[0].sticky_suspension.is_none(),
        "nothing reached the disk"
    );
    assert!(disk.entries[0].observations.is_empty());

    // (c) The safety save publishes but its durability is unconfirmed: the
    // report distinguishes visibility from durability; the disk carries it.
    let tmp = tempfile::tempdir().unwrap();
    let (path, mut lock) = make(tmp.path());
    let plan = prepare(&b, &a, &lock).unwrap();
    let err = format!(
        "{:#}",
        run(
            &mut lock,
            &plan,
            &ev,
            700,
            anchor,
            9,
            &saver_after_publish(&path)
        )
        .unwrap_err()
    );
    assert!(err.contains("durability could not be confirmed"), "{err}");
    assert!(!err.contains("confirmed durable"), "{err}");
    assert!(
        err.contains("first") && err.contains("anchor block 700"),
        "{err}"
    );
    let disk = Lockfile::load(&path).unwrap();
    assert_eq!(
        disk.entries[0].sticky_suspension.as_deref(),
        Some("quarantined")
    );
    assert_eq!(
        disk.entries[0].observations[0].context,
        b.deployment_context_id()
    );

    // (d) No adverse observation before the failure: nothing is saved and the
    // report says so.
    let tmp = tempfile::tempdir().unwrap();
    let (path, mut lock) = make(tmp.path());
    let plan = prepare(&b, &a, &lock).unwrap();
    let attempts = Cell::new(0);
    let counting = |l: &Lockfile| {
        attempts.set(attempts.get() + 1);
        l.save_phased(&path)
    };
    let ev_ok = vec![evidence(0, Ok(1)), evidence(1, Err("rpc down"))];
    let err = format!(
        "{:#}",
        run(&mut lock, &plan, &ev_ok, 700, anchor, 9, &counting).unwrap_err()
    );
    assert!(err.contains("No adverse status"), "{err}");
    assert_eq!(
        attempts.get(),
        0,
        "no safety save without adverse observations"
    );

    // (e) The success path through the same orchestration lands on disk.
    let tmp = tempfile::tempdir().unwrap();
    let (path, mut lock) = make(tmp.path());
    let plan = prepare(&b, &a, &lock).unwrap();
    let real = |l: &Lockfile| l.save_phased(&path);
    let all_ok = vec![evidence(0, Ok(1)), evidence(1, Ok(1))];
    let Outcome::Migrated(summary) =
        run(&mut lock, &plan, &all_ok, 800, anchor, 10, &real).unwrap();
    assert_eq!((summary.migrated, summary.current), (2, 2));
    let disk = Lockfile::load(&path).unwrap();
    assert!(disk
        .entries
        .iter()
        .all(|e| e.deployment_context_id == b.deployment_context_id()));
}
