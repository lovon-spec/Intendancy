//! Round-3/4 install-transaction suite. Covered here, precisely:
//! - the journal is non-replacing and carries the FULL verified manifest;
//! - a repeat install fails with the prior record untouched (both guard
//!   layers);
//! - CHILD-PROCESS CRASH TESTS (a spawned copy of this binary is hard-aborted
//!   by a failpoint) at four exact boundaries: during the journal's atomic
//!   write (pre-rename), after staged extraction (pre-publish-rename), after
//!   the publish rename but BEFORE the parent fsync, and during the
//!   finalizing lockfile write — each followed by state assertions and, where
//!   recovery is advertised, the PRODUCTION `enable_transition` recovery
//!   path (which re-establishes destination durability before clearing
//!   `pending`);
//! - single-process failure injection for a failed finalizing save (content
//!   stands, journal covers it);
//! - retargeted-parent identity: publish fails CLOSED when the bound parent
//!   pathname no longer names the bound directory, a post-publish retarget is
//!   caught by the pre-finalize identity check, and error cleanup is
//!   fd-relative (no stage residue, nothing through the planted symlink);
//! - pending entries still acquire sticky from the proven status;
//! - the provider-side builder rejects special filesystem nodes.
//!
//! NOT covered (named limits): crashes inside the kernel/filesystem itself,
//! and concurrent-attacker races during the audit walk (explicit §9
//! production blocker).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use intend::car::{self, Cid, FileSource, InstallPlan, InstallTarget, PlannedFile};
use intend::lockfile::{
    audit_state_for, enable_transition, verify_local_integrity, Entry, Lockfile, ProofContext,
};

/// The proof context matching `entry_from_plan`'s recorded binding.
fn test_ctx() -> ProofContext {
    ProofContext {
        chain_id: 100,
        registry: alloy::primitives::Address::ZERO,
        context_id: alloy::primitives::B256::ZERO,
    }
}

/// Create the small source tree (two files, one subdir with a third).
fn make_src(base: &Path) -> PathBuf {
    let src = base.join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("SKILL.md"), b"---\nname: t\n---\nbody\n").unwrap();
    std::fs::write(src.join("a.md"), b"alpha").unwrap();
    std::fs::create_dir(src.join("sub")).unwrap();
    std::fs::write(src.join("sub").join("b.md"), b"beta").unwrap();
    src
}

/// Deterministically (re)derive root/blocks/plan from an existing source tree
/// — the crash-scenario CHILD uses this over the parent-prepared tree.
fn plan_from_src(src: &Path) -> (Cid, BTreeMap<Cid, Vec<u8>>, InstallPlan) {
    let mut blocks = BTreeMap::new();
    let (root, _) = car::build_dir(src, &mut blocks).unwrap();
    let plan = car::preflight(root, &blocks).unwrap();
    (root, blocks, plan)
}

fn small_tree(base: &Path) -> (Cid, BTreeMap<Cid, Vec<u8>>, InstallPlan) {
    let src = make_src(base);
    plan_from_src(&src)
}

/// Mirror main.rs: the journal entry built from the verified plan (FULL
/// manifest — the round-3 P0-2 property).
fn entry_from_plan(plan: &InstallPlan, root: Cid, install_dir: &str, pending: bool) -> Entry {
    Entry {
        name: "t".into(),
        item_id: alloy::primitives::B256::ZERO,
        tree_cid: root.to_string_canonical(),
        chain_id: 100,
        registry: alloy::primitives::Address::ZERO,
        deployment_context_id: alloy::primitives::B256::ZERO,
        install_dir: install_dir.to_string(),
        installed_at_unix: 1,
        status_at_install: 1,
        anchor_block: 7,
        anchor_block_hash: alloy::primitives::B256::ZERO,
        anchor_state_root: alloy::primitives::B256::ZERO,
        root_block_sha256: car::hex_lower(&root.digest),
        total_bytes: plan.total_bytes,
        blocks: plan.blocks,
        files: plan.locked_files(),
        dirs: plan
            .dirs
            .iter()
            .map(|d| d.to_str().unwrap().to_owned())
            .collect(),
        audit: None,
        sticky_suspension: None,
        pending,
        migrations: Vec::new(),
        observations: Vec::new(),
    }
}

#[test]
fn journal_is_non_replacing_and_carries_the_full_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, _blocks, plan) = small_tree(tmp.path());
    let mut lock = Lockfile::load(&tmp.path().join("intend-lock.json")).unwrap();

    let journal = entry_from_plan(&plan, root, "/x/skill", true);
    // The journal is COMPLETE before anything publishes.
    assert_eq!(journal.files.len(), 3);
    assert_eq!(journal.dirs, vec!["sub".to_string()]);
    assert!(journal.total_bytes > 0 && journal.blocks > 0);
    lock.insert_new(journal.clone()).unwrap();

    // A second insertion at the same path refuses — pending case.
    let err = format!("{:#}", lock.insert_new(journal.clone()).unwrap_err());
    assert!(err.contains("PENDING journal"), "{err}");

    // Finalize (this transaction's own upsert), then a NEW install at the same
    // path refuses with the finalized-record message.
    let mut finalized = journal.clone();
    finalized.pending = false;
    lock.upsert(finalized);
    let err = format!("{:#}", lock.insert_new(journal).unwrap_err());
    assert!(err.contains("already records an install"), "{err}");
    assert_eq!(lock.entries.len(), 1);
}

#[test]
fn repeat_install_fails_with_the_prior_record_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, _plan) = small_tree(&base);
    let dest = base.join("skill");
    let lock_path = base.join("intend-lock.json");

    // First install: publish + finalized record with a sticky suspension (the
    // history a repeat install must NOT destroy).
    let (plan, published) = car::install(root, &blocks, &dest).unwrap();
    let mut entry = entry_from_plan(&plan, root, published.to_str().unwrap(), false);
    entry.sticky_suspension = Some("quarantined".into());
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(entry).unwrap();
    lock.save(&lock_path).unwrap();
    let before = std::fs::read(&lock_path).unwrap();

    // Repeat install, exactly the main.rs guard order: bind → vacancy check
    // fails BEFORE any journaling.
    let target = InstallTarget::bind(&dest).unwrap();
    let err = format!("{:#}", target.check_vacant().unwrap_err());
    assert!(err.contains("already exists"), "{err}");

    // Defense in depth: even if vacancy were skipped, the journal insertion
    // itself refuses.
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let journal = entry_from_plan(&plan, root, published.to_str().unwrap(), true);
    let err = format!("{:#}", lock.insert_new(journal).unwrap_err());
    assert!(err.contains("already records an install"), "{err}");

    // The on-disk record is byte-identical: manifest, audit history, sticky.
    assert_eq!(std::fs::read(&lock_path).unwrap(), before);
    let reloaded = Lockfile::load(&lock_path).unwrap();
    assert_eq!(
        reloaded.entries[0].sticky_suspension.as_deref(),
        Some("quarantined")
    );
    assert_eq!(reloaded.entries[0].files.len(), 3);
    verify_local_integrity(&reloaded.entries[0]).unwrap();
}

// ---------- child-process crash scenarios (failpoint-aborted) ----------

/// The CHILD body: when spawned with INTEND_CRASH_SCENARIO/-DIR set, run the
/// real transaction steps against the parent-prepared tree and let the named
/// failpoint hard-abort the process at the exact boundary. A no-op in normal
/// test runs.
#[test]
fn crash_child_scenario() {
    let (Ok(scenario), Ok(dir)) = (
        std::env::var("INTEND_CRASH_SCENARIO"),
        std::env::var("INTEND_CRASH_DIR"),
    ) else {
        return;
    };
    let base = PathBuf::from(dir);
    let (root, blocks, plan) = plan_from_src(&base.join("src"));
    let dest = base.join("skill");
    let lock_path = base.join("intend-lock.json");
    let target = InstallTarget::bind(&dest).unwrap();
    target.check_vacant().unwrap();
    let journal = entry_from_plan(&plan, root, target.path_str(), true);
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(journal.clone()).unwrap();
    match scenario.as_str() {
        // Crash DURING the journal's atomic write, before its rename.
        "journal" => {
            std::env::set_var("INTEND_FAILPOINT", "atomic-write-pre-rename");
            let _ = lock.save(&lock_path);
        }
        // Crash after staged extraction, before the publish rename.
        "staged" => {
            lock.save(&lock_path).unwrap();
            std::env::set_var("INTEND_FAILPOINT", "publish-staged");
            let _ = car::publish(&plan, &blocks, &target);
        }
        // Crash AFTER the publish rename, BEFORE the parent fsync (the
        // round-4 P0 boundary).
        "after-rename" => {
            lock.save(&lock_path).unwrap();
            std::env::set_var("INTEND_FAILPOINT", "publish-after-rename");
            let _ = car::publish(&plan, &blocks, &target);
        }
        // Crash during the FINALIZING lockfile write, before its rename.
        "finalize" => {
            lock.save(&lock_path).unwrap();
            car::publish(&plan, &blocks, &target).unwrap();
            let mut finalized = journal;
            finalized.pending = false;
            lock.upsert(finalized);
            std::env::set_var("INTEND_FAILPOINT", "atomic-write-pre-rename");
            let _ = lock.save(&lock_path);
        }
        other => panic!("unknown crash scenario {other:?}"),
    }
    unreachable!("the failpoint must have aborted the child");
}

/// Spawn this test binary as a CHILD running only `crash_child_scenario` with
/// the given scenario, assert it died of SIGABRT (a real crash, not a test
/// failure), and hand the surviving state back.
fn run_crash_scenario(scenario: &str) -> (PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    make_src(&base);
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "crash_child_scenario",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("INTEND_CRASH_SCENARIO", scenario)
        .env("INTEND_CRASH_DIR", &base)
        .env_remove("INTEND_FAILPOINT")
        .output()
        .unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        out.status.signal(),
        Some(libc::SIGABRT),
        "child (scenario {scenario}) must die at the failpoint; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    (base, tmp)
}

#[test]
fn crash_during_journal_write_leaves_no_record_and_a_clean_retry() {
    let (base, _tmp) = run_crash_scenario("journal");
    let lock_path = base.join("intend-lock.json");
    // The journal's atomic write never renamed: no lockfile, no destination.
    assert!(!lock_path.exists());
    assert!(!base.join("skill").exists());
    // Recovery is a plain retry: the full transaction now completes.
    let (root, blocks, plan) = plan_from_src(&base.join("src"));
    let target = InstallTarget::bind(&base.join("skill")).unwrap();
    target.check_vacant().unwrap();
    let journal = entry_from_plan(&plan, root, target.path_str(), true);
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(journal.clone()).unwrap();
    lock.save(&lock_path).unwrap();
    car::publish(&plan, &blocks, &target).unwrap();
    let mut finalized = journal;
    finalized.pending = false;
    lock.upsert(finalized);
    lock.save(&lock_path).unwrap();
    verify_local_integrity(&Lockfile::load(&lock_path).unwrap().entries[0]).unwrap();
}

#[test]
fn crash_during_staging_leaves_journal_plus_orphan_stage_fail_closed() {
    let (base, _tmp) = run_crash_scenario("staged");
    let lock_path = base.join("intend-lock.json");
    // Durable state: full-manifest pending journal; NO destination; ONE orphan
    // stage directory (the documented residual of an extraction crash — safe
    // to delete once no intend process is running; cleanup is manual).
    let mut lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(lock.entries.len(), 1);
    let entry = &mut lock.entries[0];
    assert!(entry.pending);
    assert_eq!(entry.files.len(), 3);
    assert!(!base.join("skill").exists());
    let stages: Vec<_> = std::fs::read_dir(&base)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".intend-stage-")
        })
        .collect();
    assert_eq!(stages.len(), 1, "exactly one orphaned stage");
    // Fail-closed surfaces: audit=incomplete, integrity names the missing
    // dir, the PRODUCTION enable transition refuses, reinstall at the same
    // path refuses (stale-journal removal is deliberate and manual).
    assert_eq!(audit_state_for(entry, 1, false).0, "incomplete");
    let err = format!("{:#}", verify_local_integrity(entry).unwrap_err());
    assert!(err.contains("missing"), "{err}");
    let err = format!(
        "{:#}",
        enable_transition(entry, &test_ctx(), 1).unwrap_err()
    );
    assert!(err.contains("integrity"), "{err}");
    let journal2 = entry_from_plan(
        &plan_from_src(&base.join("src")).2,
        plan_from_src(&base.join("src")).0,
        &lock.entries[0].install_dir.clone(),
        true,
    );
    let err = format!("{:#}", lock.insert_new(journal2).unwrap_err());
    assert!(err.contains("PENDING journal"), "{err}");
}

#[test]
fn crash_between_rename_and_parent_fsync_recovers_via_production_enable() {
    // THE round-4 P0 boundary: content is VISIBLE but its dirent was never
    // fsynced; the pending journal is the durable truth.
    let (base, _tmp) = run_crash_scenario("after-rename");
    let lock_path = base.join("intend-lock.json");
    let dest = base.join("skill");
    assert!(dest.join("SKILL.md").exists(), "content visible post-crash");
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let entry = &mut lock.entries[0];
    assert!(entry.pending);
    verify_local_integrity(entry).unwrap();
    assert_eq!(audit_state_for(entry, 1, true).0, "incomplete");
    // PRODUCTION recovery — the same `enable_transition` `intend enable`
    // runs: it re-binds the destination and FSYNCS ITS PARENT (making the
    // rename durable) before clearing pending.
    let cleared = enable_transition(entry, &test_ctx(), 1).unwrap();
    assert_eq!(cleared, None);
    assert!(!entry.pending);
    lock.save(&lock_path).unwrap();
    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(
        audit_state_for(&lock.entries[0], 1, true),
        ("current", None)
    );
    verify_local_integrity(&lock.entries[0]).unwrap();
}

#[test]
fn crash_during_finalizing_save_recovers_via_production_enable() {
    let (base, _tmp) = run_crash_scenario("finalize");
    let lock_path = base.join("intend-lock.json");
    // The finalizing write never renamed: the durable lockfile still holds
    // the PENDING journal; content is fully published.
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let entry = &mut lock.entries[0];
    assert!(entry.pending, "durable state is the pending journal");
    verify_local_integrity(entry).unwrap();
    let cleared = enable_transition(entry, &test_ctx(), 1).unwrap();
    assert_eq!(cleared, None);
    lock.save(&lock_path).unwrap();
    assert_eq!(
        audit_state_for(&Lockfile::load(&lock_path).unwrap().entries[0], 1, true),
        ("current", None)
    );
}

// ---------- single-process failure injection + production enable guards ----------

#[test]
fn crash_before_publish_leaves_a_recoverable_full_manifest_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, _blocks, plan) = small_tree(&base);
    let dest = base.join("skill");
    let lock_path = base.join("intend-lock.json");

    // Transaction steps 1-3 (bind, checks, journal save)… then crash: no
    // publish ever happens.
    let target = InstallTarget::bind(&dest).unwrap();
    target.check_vacant().unwrap();
    let journal = entry_from_plan(&plan, root, target.path_str(), true);
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(journal).unwrap();
    lock.save(&lock_path).unwrap();
    drop(target);

    // Recovery surface, stated precisely: the journal is pending WITH the
    // manifest; audit reports "incomplete"; integrity says exactly what's
    // wrong (the directory is missing — nothing was published); the
    // PRODUCTION enable transition refuses (no content to verify); and a
    // blind reinstall at the same path is refused by the journal's presence.
    // Deliberately manual last step: removing the stale journal entry.
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let entry = &mut lock.entries[0];
    assert!(entry.pending);
    assert_eq!(entry.files.len(), 3);
    assert_eq!(audit_state_for(entry, 1, false).0, "incomplete");
    let err = format!("{:#}", verify_local_integrity(entry).unwrap_err());
    assert!(err.contains("missing"), "{err}");
    let err = format!(
        "{:#}",
        enable_transition(entry, &test_ctx(), 1).unwrap_err()
    );
    assert!(err.contains("integrity"), "{err}");
    let journal2 = entry_from_plan(&plan, root, &lock.entries[0].install_dir.clone(), true);
    let err = format!("{:#}", lock.insert_new(journal2).unwrap_err());
    assert!(err.contains("PENDING journal"), "{err}");
}

#[test]
fn crash_after_publish_is_finalized_by_the_enable_transition() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, plan) = small_tree(&base);
    let dest = base.join("skill");
    let lock_path = base.join("intend-lock.json");

    // Steps 1-4 (journal saved, content published)… then crash BEFORE the
    // finalizing save (single-process variant; the child-process variant
    // above kills even earlier, between rename and parent fsync).
    let target = InstallTarget::bind(&dest).unwrap();
    target.check_vacant().unwrap();
    let journal = entry_from_plan(&plan, root, target.path_str(), true);
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(journal).unwrap();
    lock.save(&lock_path).unwrap();
    let published = car::publish(&plan, &blocks, &target).unwrap();
    drop(lock); // crash: the finalizing save never runs

    // Recovery through the PRODUCTION path.
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let entry = &mut lock.entries[0];
    assert!(entry.pending);
    assert_eq!(entry.install_dir, published.to_str().unwrap());
    assert!(!entry.files.is_empty(), "journal must carry the manifest");
    verify_local_integrity(entry).unwrap();
    assert_eq!(audit_state_for(entry, 1, true).0, "incomplete");
    let cleared = enable_transition(entry, &test_ctx(), 1).unwrap();
    assert_eq!(cleared, None);
    assert!(!entry.pending);
    lock.save(&lock_path).unwrap();
    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(
        audit_state_for(&lock.entries[0], 1, true),
        ("current", None)
    );
    verify_local_integrity(&lock.entries[0]).unwrap();
}

#[test]
fn enable_transition_guards_fire_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, plan) = small_tree(&base);

    // Nothing to enable.
    let mut e = entry_from_plan(&plan, root, "/x/skill", false);
    let err = format!(
        "{:#}",
        enable_transition(&mut e, &test_ctx(), 1).unwrap_err()
    );
    assert!(err.contains("nothing to enable"), "{err}");

    // Pending journal WITHOUT a manifest (foreign/legacy): refused.
    let mut e = entry_from_plan(&plan, root, "/x/skill", true);
    e.files.clear();
    let err = format!(
        "{:#}",
        enable_transition(&mut e, &test_ctx(), 1).unwrap_err()
    );
    assert!(err.contains("without a manifest"), "{err}");

    // Real published content, but the fresh proof is not Registered.
    let dest = base.join("skill");
    let (plan2, published) = car::install(root, &blocks, &dest).unwrap();
    let mut e = entry_from_plan(&plan2, root, published.to_str().unwrap(), true);
    let err = format!(
        "{:#}",
        enable_transition(&mut e, &test_ctx(), 3).unwrap_err()
    );
    assert!(err.contains("Registered"), "{err}");
    assert!(e.pending, "a refused transition must not clear pending");

    // Sticky-only re-enable (non-pending) returns the cleared suspension.
    let mut e = entry_from_plan(&plan2, root, published.to_str().unwrap(), false);
    e.sticky_suspension = Some("revoked".into());
    assert_eq!(
        enable_transition(&mut e, &test_ctx(), 1).unwrap(),
        Some("revoked".into())
    );
}

#[test]
fn finalize_failure_keeps_published_content_and_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, plan) = small_tree(&base);
    let dest = base.join("skill");
    let lockdir = base.join("lockdir");
    std::fs::create_dir(&lockdir).unwrap();
    let lock_path = lockdir.join("intend-lock.json");

    let target = InstallTarget::bind(&dest).unwrap();
    target.check_vacant().unwrap();
    let journal = entry_from_plan(&plan, root, target.path_str(), true);
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(journal.clone()).unwrap();
    lock.save(&lock_path).unwrap();
    car::publish(&plan, &blocks, &target).unwrap();

    // Inject the finalization failure: the lockfile's directory becomes
    // unwritable, so the atomic finalizing save cannot create its temp file.
    let mut perms = std::fs::metadata(&lockdir).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o500);
    std::fs::set_permissions(&lockdir, perms.clone()).unwrap();
    let mut finalized = journal;
    finalized.pending = false;
    lock.upsert(finalized);
    let err = lock.save(&lock_path).unwrap_err();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o700);
    std::fs::set_permissions(&lockdir, perms).unwrap();
    let _ = err; // the command surfaces this with the audit/enable guidance

    // Published content STANDS (never deleted on finalization failure), and
    // the durable journal still covers it — the enable recovery applies.
    assert!(dest.join("SKILL.md").exists());
    let reloaded = Lockfile::load(&lock_path).unwrap();
    assert!(reloaded.entries[0].pending);
    verify_local_integrity(&reloaded.entries[0]).unwrap();
}

#[test]
fn pending_entries_still_acquire_sticky_from_the_proven_status() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, _blocks, plan) = small_tree(tmp.path());
    let mut e = entry_from_plan(&plan, root, "/x/skill", true);

    // Quarantine observed while the entry is pending: display stays
    // fail-closed "incomplete", but the sticky suspension IS acquired…
    assert_eq!(
        audit_state_for(&e, 3, true),
        ("incomplete", Some("quarantined".into()))
    );
    assert_eq!(
        audit_state_for(&e, 0, true),
        ("incomplete", Some("revoked".into()))
    );
    // …so once the journal is finalized, a return to Registered still demands
    // the explicit re-enable instead of slipping straight to "current".
    e.sticky_suspension = Some("quarantined".into());
    e.pending = false;
    assert_eq!(
        audit_state_for(&e, 1, true),
        ("reenable-required", Some("quarantined".into()))
    );
}

// ---------- retargeted-parent identity (round-4 P1) ----------

#[test]
fn retargeted_parent_fails_publish_closed_with_no_stale_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (_root, blocks, plan) = small_tree(&base);
    let parent = base.join("realparent");
    let moved = base.join("moved");
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(&elsewhere).unwrap();

    // Bind, then RETARGET the parent path (real directory moved away, symlink
    // to an attacker-chosen tree planted in its place): publish must fail
    // CLOSED at the identity check — never writing through the symlink and
    // never landing content under a pathname that no longer names the bound
    // directory.
    let target = InstallTarget::bind(&parent.join("skill")).unwrap();
    std::fs::rename(&parent, &moved).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &parent).unwrap();
    let err = format!("{:#}", car::publish(&plan, &blocks, &target).unwrap_err());
    assert!(err.contains("retargeted"), "{err}");
    assert!(!elsewhere.join("skill").exists());
    assert!(!moved.join("skill").exists());
}

#[test]
fn retarget_after_publish_is_caught_before_finalization() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, plan) = small_tree(&base);
    let parent = base.join("realparent");
    let moved = base.join("moved");
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(&elsewhere).unwrap();

    // Successful publish into the bound parent…
    let target = InstallTarget::bind(&parent.join("skill")).unwrap();
    let published = car::publish(&plan, &blocks, &target).unwrap();
    assert_eq!(published, parent.join("skill"));
    // …then the retarget happens BETWEEN publish and finalization: the
    // pre-finalize identity check (main runs exactly this) fails closed, so
    // the journal stays pending instead of finalizing a stale identity.
    std::fs::rename(&parent, &moved).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &parent).unwrap();
    let err = format!("{:#}", target.verify_identity().unwrap_err());
    assert!(err.contains("retargeted"), "{err}");
    // Content is where the bound directory went; the recorded pathname now
    // resolves through the symlink to nothing — audit fails closed.
    assert!(moved.join("skill").join("SKILL.md").exists());
    let entry = entry_from_plan(&plan, root, published.to_str().unwrap(), true);
    let err = format!("{:#}", verify_local_integrity(&entry).unwrap_err());
    assert!(err.contains("missing"), "{err}");
}

#[test]
fn failed_staging_cleanup_is_fd_relative_and_leaves_no_residue() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    // A crafted plan whose second file lies under a directory the plan never
    // creates: staging populates real content (dir + one file), then fails —
    // the fd-relative cleanup must recursively remove the whole stage.
    let content = b"x".to_vec();
    let plan = InstallPlan {
        dirs: vec![PathBuf::from("d")],
        files: vec![
            PlannedFile {
                rel: PathBuf::from("d/ok.md"),
                source: FileSource::Inline(content.clone()),
                bytes: 1,
                sha256: car::hex_lower(&[0u8; 32]),
            },
            PlannedFile {
                rel: PathBuf::from("missing/never.md"),
                source: FileSource::Inline(content),
                bytes: 1,
                sha256: car::hex_lower(&[0u8; 32]),
            },
        ],
        total_bytes: 2,
        blocks: 0,
    };
    let target = InstallTarget::bind(&base.join("skill")).unwrap();
    let err = format!(
        "{:#}",
        car::publish(&plan, &BTreeMap::new(), &target).unwrap_err()
    );
    assert!(err.contains("creating staged file"), "{err}");
    assert!(!base.join("skill").exists());
    let residue: Vec<_> = std::fs::read_dir(&base)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".intend-stage-")
        })
        .collect();
    assert!(
        residue.is_empty(),
        "stage must be fully cleaned, fd-relative"
    );
}

#[test]
fn build_dir_rejects_special_nodes_instead_of_reading_them() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("a.md"), b"alpha").unwrap();
    let fifo = src.join("pipe");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);

    // Reading the FIFO would hang forever; the builder must fail typed first.
    let mut blocks = BTreeMap::new();
    let err = format!("{:#}", car::build_dir(&src, &mut blocks).unwrap_err());
    assert!(err.contains("node kind"), "{err}");
}

#[test]
fn enable_refuses_a_proof_from_another_registry_or_chain() {
    // Round-5 P0: an entry installed under registry A must never have its
    // pending/sticky state cleared by a Registered proof for the same itemID
    // in registry B (or another chain). The proof CONTEXT is bound into the
    // transition itself and checked before anything else.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, blocks, _plan) = small_tree(&base);
    let dest = base.join("skill");
    let lock_path = base.join("intend-lock.json");
    let (plan, published) = car::install(root, &blocks, &dest).unwrap();
    let mut entry = entry_from_plan(&plan, root, published.to_str().unwrap(), true);
    entry.sticky_suspension = Some("revoked".into());
    let mut lock = Lockfile::load(&lock_path).unwrap();
    lock.insert_new(entry).unwrap();
    lock.save(&lock_path).unwrap();
    let before = std::fs::read(&lock_path).unwrap();

    let other_registry: alloy::primitives::Address = "0x00000000000000000000000000000000000000aa"
        .parse()
        .unwrap();
    let mut lock = Lockfile::load(&lock_path).unwrap();
    let entry = &mut lock.entries[0];
    // Wrong registry, right chain — refused before any mutation (the context
    // id, which covers the numeric fields, differs accordingly).
    let mut ctx = test_ctx();
    ctx.registry = other_registry;
    ctx.context_id = alloy::primitives::B256::repeat_byte(0xaa);
    let err = format!("{:#}", enable_transition(entry, &ctx, 1).unwrap_err());
    assert!(err.contains("deployment context"), "{err}");
    // Wrong chain, right registry — refused too.
    let mut ctx = test_ctx();
    ctx.chain_id = 999;
    ctx.context_id = alloy::primitives::B256::repeat_byte(0xbb);
    let err = format!("{:#}", enable_transition(entry, &ctx, 1).unwrap_err());
    assert!(err.contains("deployment context"), "{err}");
    // Round-6: SAME chain and registry but a DIFFERENT deployment context
    // (other genesis / codehash / policy pins / test mode) — refused too.
    let mut ctx = test_ctx();
    ctx.context_id = alloy::primitives::B256::repeat_byte(0xcc);
    let err = format!("{:#}", enable_transition(entry, &ctx, 1).unwrap_err());
    assert!(err.contains("deployment context"), "{err}");
    // Entry state untouched: still pending, sticky intact.
    assert!(entry.pending);
    assert_eq!(entry.sticky_suspension.as_deref(), Some("revoked"));
    // And nothing was persisted: the lockfile bytes are identical.
    assert_eq!(std::fs::read(&lock_path).unwrap(), before);
    // The CORRECT binding still works.
    let cleared = enable_transition(entry, &test_ctx(), 1).unwrap();
    assert_eq!(cleared, Some("revoked".into()));
    assert!(!entry.pending);
}
