//! Policy-version migration suite (`intend migrate`): the successor relation
//! gates it, entries move all-or-nothing, sticky suspensions and pending
//! journals are preserved, fresh states are recorded, and the lockfile keeps
//! every migration record. The network-free plan/apply pair is exercised with
//! fixture profiles and hand-built entries; the CLI's own migrate command
//! feeds it the anchor and per-entry point checks.

use alloy::primitives::{Address, Bytes, B256};
use intend::lockfile::{Entry, Lockfile, ProofContext};
use intend::migrate::{apply, prepare, Checked};
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
    }
}

fn lock_with(entries: Vec<Entry>) -> Lockfile {
    Lockfile {
        version: intend::lockfile::LOCKFILE_VERSION,
        entries,
    }
}

fn registered(plan: &intend::migrate::MigrationPlan) -> Vec<Checked> {
    plan.to_migrate
        .iter()
        .map(|&index| Checked {
            index,
            fresh_status: 1,
            local_intact: true,
        })
        .collect()
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
            (m.anchor_block, m.status, m.state.as_str()),
            (1234, 1, "current")
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
        Checked {
            index: 0,
            fresh_status: 1,
            local_intact: true,
        },
        Checked {
            index: 1,
            fresh_status: 3,
            local_intact: true,
        },
        Checked {
            index: 2,
            fresh_status: 0,
            local_intact: true,
        },
        Checked {
            index: 3,
            fresh_status: 1,
            local_intact: true,
        },
        Checked {
            index: 4,
            fresh_status: 1,
            local_intact: true,
        },
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
    assert_eq!(summary.suspended(), 2);
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
    let wrong = [Checked {
        index: 0,
        fresh_status: 1,
        local_intact: true,
    }];
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
    // A v2 lockfile written before this feature has no `migrations` field.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("intend-lock.json");
    let a = profile_a();
    let mut lock = lock_with(vec![entry("one", "/skills/one", &a.proof_context())]);
    lock.save(&path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("migrations"),
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
