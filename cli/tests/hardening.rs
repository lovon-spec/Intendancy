//! Round-2 hardening suite: HTTP-boundary snapshot caps with streaming gzip
//! sniff and one-at-a-time failover; Multicall cardinality; the exact-max-items
//! slot-cap boundary; audit integrity edge cases (symlinked root, special
//! nodes, size-first, directory-set drift); and the install-transaction
//! journal states. Offline (local TCP only).

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::build_fixture;
use intend::lockfile::{audit_state_for, verify_local_integrity, Entry, LockedFile};
use intend::snapshot::{verify, Limits};

/// Minimal one-shot HTTP server: serves `body` to every connection, counting
/// hits. Returns (base_url, hit counter, join guard via leaked thread).
fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits2 = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            hits2.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    (format!("http://{addr}/snap"), hits)
}

fn tiny_limits(compressed: u64, decoded: u64) -> Limits {
    Limits {
        max_compressed_bytes: compressed,
        max_decoded_bytes: decoded,
        ..Limits::default()
    }
}

#[tokio::test]
async fn snapshot_fetch_caps_apply_per_encoding_at_the_http_boundary() {
    // RAW body over the decoded cap: rejected mid-stream.
    let (url, _) = serve(vec![b'{'; 4096]);
    let err = intend::fetch::fetch_snapshot(&url, &tiny_limits(1 << 20, 1024))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("cap"), "{err:#}");

    // GZIP body over the COMPRESSED cap (but under the decoded cap): the sniff
    // must select the compressed cap.
    let mut gz = vec![0x1f, 0x8b];
    gz.extend_from_slice(&[0u8; 4096]);
    let (url, _) = serve(gz);
    let err = intend::fetch::fetch_snapshot(&url, &tiny_limits(1024, 1 << 20))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("cap"), "{err:#}");

    // GZIP body UNDER the compressed cap passes the boundary even though it
    // exceeds a hypothetical tighter raw cap.
    let (raw_snap, _) = {
        let f = build_fixture();
        (serde_json::to_vec(&f.snapshot).unwrap(), ())
    };
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw_snap).unwrap();
    let gz = enc.finish().unwrap();
    let gz_len = gz.len() as u64;
    let (url, _) = serve(gz);
    let bytes = intend::fetch::fetch_snapshot(&url, &tiny_limits(gz_len + 10, 1 << 26))
        .await
        .unwrap();
    let (parsed, _) = intend::snapshot::read_snapshot_bounded(&bytes, &Limits::default()).unwrap();
    assert_eq!(parsed.item_count, 4);
}

#[tokio::test]
async fn snapshot_failover_is_lazy_one_candidate_at_a_time() {
    // Source 1 serves a valid snapshot; source 2 must NEVER be contacted when
    // the caller stops at the first success (one-candidate-at-a-time rule).
    let f = build_fixture();
    let raw = serde_json::to_vec(&f.snapshot).unwrap();
    let (url1, hits1) = serve(raw);
    let (url2, hits2) = serve(vec![b'x'; 8]);
    let limits = Limits::default();
    for url in [&url1, &url2] {
        let bytes = intend::fetch::fetch_snapshot(url, &limits).await.unwrap();
        if let Ok((snap, _)) = intend::snapshot::read_snapshot_bounded(&bytes, &limits) {
            // "verify" step of the loop: full §6 against the fixture profile.
            verify(&snap, &f.profile, &limits).unwrap();
            break; // success: later sources are never contacted
        }
    }
    assert_eq!(hits1.load(Ordering::SeqCst), 1);
    assert_eq!(
        hits2.load(Ordering::SeqCst),
        0,
        "second source never contacted"
    );

    // And when the FIRST source fails verification (garbage), the second is
    // consulted — failover past verification failures, still sequential.
    let (bad_url, bad_hits) = serve(vec![b'x'; 8]);
    let raw = serde_json::to_vec(&f.snapshot).unwrap();
    let (good_url, good_hits) = serve(raw);
    let mut verified = false;
    for url in [&bad_url, &good_url] {
        let bytes = intend::fetch::fetch_snapshot(url, &limits).await.unwrap();
        match intend::snapshot::read_snapshot_bounded(&bytes, &limits) {
            Ok((snap, _)) if verify(&snap, &f.profile, &limits).is_ok() => {
                verified = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(verified);
    assert_eq!(bad_hits.load(Ordering::SeqCst), 1);
    assert_eq!(good_hits.load(Ordering::SeqCst), 1);
}

#[test]
fn multicall_cardinality_guard() {
    assert!(intend::chain::check_batch_cardinality(3, 3).is_ok());
    assert!(
        intend::chain::check_batch_cardinality(3, 4).is_err(),
        "extra result"
    );
    assert!(
        intend::chain::check_batch_cardinality(3, 2).is_err(),
        "missing result"
    );
}

#[test]
fn slot_cap_boundary_at_exact_max_items() {
    // 4 items ⇒ 2N+2 = 10 slot proofs, plus the arbitrator slot and the three
    // extra-data words of a 64-byte pin, plus the governor slot = 15. With
    // max_items = 4 EXACTLY, the healthy fixture must verify (the old 2N+1 cap
    // wrongly rejected it).
    let f = build_fixture();
    let limits = Limits {
        max_items: 4,
        ..Limits::default()
    };
    let stats = verify(&f.snapshot, &f.profile, &limits).expect("boundary must pass");
    assert_eq!(stats.slot_proofs_checked, 15);
}

fn entry_for(dir: &std::path::Path, files: Vec<LockedFile>, dirs: Vec<String>) -> Entry {
    Entry {
        name: "t".into(),
        item_id: alloy::primitives::B256::ZERO,
        tree_cid: "t".into(),
        chain_id: 100,
        registry: alloy::primitives::Address::ZERO,
        deployment_context_id: alloy::primitives::B256::ZERO,
        install_dir: dir.to_string_lossy().into_owned(),
        installed_at_unix: 0,
        status_at_install: 1,
        anchor_block: 0,
        anchor_block_hash: alloy::primitives::B256::ZERO,
        anchor_state_root: alloy::primitives::B256::ZERO,
        root_block_sha256: String::new(),
        total_bytes: 0,
        blocks: 0,
        files,
        dirs,
        audit: None,
        sticky_suspension: None,
        pending: false,
        migrations: Vec::new(),
    }
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

#[test]
fn integrity_rejects_symlinked_root_special_nodes_and_dir_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::create_dir(real.join("sub")).unwrap();
    std::fs::write(real.join("a.md"), b"alpha").unwrap();
    let files = vec![LockedFile {
        path: "a.md".into(),
        bytes: 5,
        sha256: sha256_hex(b"alpha"),
    }];
    let dirs = vec!["sub".into()];

    // Intact baseline.
    verify_local_integrity(&entry_for(&real, files.clone(), dirs.clone())).unwrap();

    // Root is a SYMLINK to an identical tree: must be refused outright.
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let err = verify_local_integrity(&entry_for(&alias, files.clone(), dirs.clone())).unwrap_err();
    assert!(format!("{err:#}").contains("SYMLINK"), "{err:#}");

    // A FIFO in the tree: typed rejection, no hanging read.
    let fifo = real.join("pipe");
    let c = std::ffi::CString::new(fifo.to_string_lossy().into_owned()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let err = verify_local_integrity(&entry_for(&real, files.clone(), dirs.clone())).unwrap_err();
    assert!(format!("{err:#}").contains("node kind"), "{err:#}");
    std::fs::remove_file(&fifo).unwrap();

    // Oversized replacement: rejected on the SIZE check (metadata-first).
    std::fs::write(real.join("a.md"), vec![b'x'; 1000]).unwrap();
    let err = verify_local_integrity(&entry_for(&real, files.clone(), dirs.clone())).unwrap_err();
    assert!(format!("{err:#}").contains("bytes"), "{err:#}");
    std::fs::write(real.join("a.md"), b"alpha").unwrap();

    // Extra EMPTY directory appears: directory-set drift is detected.
    std::fs::create_dir(real.join("extra")).unwrap();
    let err = verify_local_integrity(&entry_for(&real, files.clone(), dirs.clone())).unwrap_err();
    assert!(format!("{err:#}").contains("extra directory"), "{err:#}");
    std::fs::remove_dir(real.join("extra")).unwrap();

    // Expected directory removed.
    std::fs::remove_dir(real.join("sub")).unwrap();
    let err = verify_local_integrity(&entry_for(&real, files, dirs)).unwrap_err();
    assert!(format!("{err:#}").contains("missing"), "{err:#}");
}

// NOTE (round-4): the pre-materialization cap contract is proven at the READER
// layer by the `fetch` unit tests — a counting reader shows consumption stops
// at exactly cap + 1 bytes for both encodings, degenerate caps included. The
// async chunk-admission helper the removed test exercised is gone, because a
// frame handed over by an async stream is already materialized before any
// check can run; the body is now consumed via a blocking take-bounded reader.
// The server-based tests in this file exercise the same caps end-to-end at
// the HTTP boundary.
/// One-shot HTTP server for the RPC-transport cap tests: serves `body`,
/// optionally WITHOUT a content-length header (close-delimited), to exercise
/// both the advertised-length precheck and the streaming cap.
fn serve_rpc(body: Vec<u8>, content_length: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let header = if content_length {
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
            } else {
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n"
                    .to_string()
            };
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn capped_rpc_transport_rejects_oversized_responses() {
    use alloy::providers::Provider;
    // A syntactically plausible but oversized JSON-RPC body (never parsed —
    // the cap fires first).
    let mut body = b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":\"0x1\"}".to_vec();
    body.resize(200 * 1024, b' ');

    // Advertised content-length over the cap: rejected by the precheck.
    let url = serve_rpc(body.clone(), true);
    let provider = intend::transport::capped_provider_with_cap(&url, 64 * 1024).unwrap();
    let err = provider.get_block_number().await.unwrap_err();
    assert!(format!("{err}").contains("cap"), "{err}");

    // No content-length (close-delimited stream): rejected mid-stream by the
    // chunk-loop cap.
    let url = serve_rpc(body, false);
    let provider = intend::transport::capped_provider_with_cap(&url, 64 * 1024).unwrap();
    let err = provider.get_block_number().await.unwrap_err();
    assert!(format!("{err}").contains("cap"), "{err}");
}

/// Server that answers the first `fail_first` requests with a transient
/// status and every later one with `body`.
fn serve_rpc_flaky(status_line: &'static str, fail_first: usize, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut seen = 0usize;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            seen += 1;
            if seen <= fail_first {
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                );
                continue;
            }
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn capped_rpc_transport_retries_transient_statuses_but_not_final_ones() {
    use alloy::providers::Provider;
    let ok = b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":\"0x10\"}".to_vec();

    // One 408 then success: the read is idempotent, so the retry answers.
    let url = serve_rpc_flaky("408 Request Timeout", 1, ok.clone());
    let provider = intend::transport::capped_provider_with_cap(&url, 64 * 1024).unwrap();
    assert_eq!(provider.get_block_number().await.unwrap(), 16);

    // Transient on every attempt: the bounded retry gives up with the status.
    let url = serve_rpc_flaky("503 Service Unavailable", usize::MAX, ok.clone());
    let provider = intend::transport::capped_provider_with_cap(&url, 64 * 1024).unwrap();
    let err = provider.get_block_number().await.unwrap_err();
    assert!(format!("{err}").contains("HTTP 503"), "{err}");

    // A non-transient status is final on the first answer.
    let url = serve_rpc_flaky("404 Not Found", 1, ok);
    let provider = intend::transport::capped_provider_with_cap(&url, 64 * 1024).unwrap();
    let err = provider.get_block_number().await.unwrap_err();
    assert!(format!("{err}").contains("HTTP 404"), "{err}");
}

#[test]
fn pending_journal_entries_audit_as_incomplete_until_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let mut e = entry_for(tmp.path(), Vec::new(), Vec::new());
    e.pending = true;
    // Incomplete regardless of a healthy fresh status and intact bytes.
    assert_eq!(audit_state_for(&e, 1, true), ("incomplete", None));
    assert_eq!(audit_state_for(&e, 0, true).0, "incomplete");
    // Once finalized (pending cleared), normal classification resumes.
    e.pending = false;
    assert_eq!(audit_state_for(&e, 1, true), ("current", None));
    // Journal round-trips through the lockfile serialization.
    e.pending = true;
    let json = serde_json::to_string(&e).unwrap();
    let back: Entry = serde_json::from_str(&json).unwrap();
    assert!(back.pending);
}

#[test]
fn catalog_binding_mismatch_is_rejected() {
    // Round-5: a state dir last verified for registry A must not serve as the
    // catalog (display OR name/completeness resolution) while profile B is
    // active — the binding is checked right after load.
    use intend::store::{Catalog, CatalogMeta};
    let f = build_fixture();
    let tmp = tempfile::tempdir().unwrap();
    let raw = serde_json::to_vec(&f.snapshot).unwrap();
    use sha2::{Digest, Sha256};
    let ctx = alloy::primitives::B256::repeat_byte(0x11);
    let meta = CatalogMeta {
        deployment_context_id: ctx,
        verified_at_unix: 1,
        anchor_block: f.profile.anchor_block,
        anchor_block_hash: f.profile.anchor_block_hash,
        anchor_state_root: f.profile.anchor_state_root,
        anchor_mode: "test".into(),
        items: f.snapshot.item_count,
        snapshot_sha256: Sha256::digest(&raw)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    Catalog::save(tmp.path(), &meta, &f.snapshot).unwrap();
    let catalog = Catalog::load(tmp.path()).unwrap();
    // The verified binding passes…
    catalog
        .assert_bound_to(f.profile.chain_id, f.profile.registry, ctx)
        .unwrap();
    // …a different registry (same chain) and a different chain both reject.
    let other: alloy::primitives::Address = "0x00000000000000000000000000000000000000aa"
        .parse()
        .unwrap();
    let err = format!(
        "{:#}",
        catalog
            .assert_bound_to(f.profile.chain_id, other, ctx)
            .unwrap_err()
    );
    assert!(err.contains("active profile pins"), "{err}");
    let err = format!(
        "{:#}",
        catalog
            .assert_bound_to(f.profile.chain_id + 1, f.profile.registry, ctx)
            .unwrap_err()
    );
    assert!(err.contains("active profile pins"), "{err}");
    // Round-6: the SAME numeric binding under a DIFFERENT deployment context
    // (other genesis / codehash / policy pins / test mode) rejects too.
    let err = format!(
        "{:#}",
        catalog
            .assert_bound_to(
                f.profile.chain_id,
                f.profile.registry,
                alloy::primitives::B256::repeat_byte(0x22)
            )
            .unwrap_err()
    );
    assert!(err.contains("DIFFERENT deployment context"), "{err}");
}

#[test]
fn highwater_marks_are_genesis_qualified_and_v1_files_reject() {
    use intend::anchor::QuorumAnchor;
    use intend::store::HighWater;
    let tmp = tempfile::tempdir().unwrap();
    let reg = alloy::primitives::Address::ZERO;
    let g_a = alloy::primitives::B256::repeat_byte(0xaa);
    let g_b = alloy::primitives::B256::repeat_byte(0xbb);
    let anchor = |n: u64| QuorumAnchor {
        block_number: n,
        block_hash: alloy::primitives::B256::repeat_byte(n as u8),
        state_root: alloy::primitives::B256::ZERO,
        timestamp: 0,
        sources: 2,
    };
    // Advance under genesis A to height 100…
    let mut hw = HighWater::load(tmp.path()).unwrap();
    assert!(hw.observe(100, g_a, reg, &anchor(100)).unwrap());
    // …the SAME chain id + registry under genesis B is an INDEPENDENT mark:
    // an older height there is not a rollback.
    assert!(hw.observe(100, g_b, reg, &anchor(50)).unwrap());
    // Under genesis A, an older height still rejects.
    let err = format!("{:#}", hw.observe(100, g_a, reg, &anchor(99)).unwrap_err());
    assert!(err.contains("rollback refused"), "{err}");

    // A version-1 (pre-context) high-water file is ambiguous: fail closed.
    let tmp2 = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp2.path().join("highwater.json"),
        b"{\"version\":1,\"marks\":{}}",
    )
    .unwrap();
    let err = format!("{:#}", HighWater::load(tmp2.path()).unwrap_err());
    assert!(err.contains("version 1"), "{err}");
}

#[test]
fn lockfile_resolver_rejects_hard_links_and_reserved_lock_names() {
    use intend::lockfile::{resolve_lockfile_path, Lockfile};
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();

    // Hard-link aliases cannot be collapsed to one identity: rejected.
    let real = base.join("real-lock.json");
    Lockfile::load(&real).unwrap().save(&real).unwrap();
    let hard = base.join("hard-alias.json");
    std::fs::hard_link(&real, &hard).unwrap();
    for p in [&real, &hard] {
        let err = format!("{:#}", resolve_lockfile_path(p).unwrap_err());
        assert!(err.contains("link count"), "{err}");
    }
    std::fs::remove_file(&hard).unwrap();
    resolve_lockfile_path(&real).unwrap();

    // Names ending in .lock are RESERVED for companion flocks — a data
    // lockfile there would collide with a sibling lockfile's lock path.
    let err = format!(
        "{:#}",
        resolve_lockfile_path(&base.join("state.lock")).unwrap_err()
    );
    assert!(err.contains("reserved"), "{err}");
    let existing = base.join("data.lock");
    std::fs::write(&existing, b"{}").unwrap();
    let err = format!("{:#}", resolve_lockfile_path(&existing).unwrap_err());
    assert!(err.contains("reserved"), "{err}");
}

#[test]
fn lockfile_symlink_aliases_converge_on_one_identity() {
    // Round-5: the lockfile identity is resolved BEFORE the data path and its
    // companion .lock are derived, so an alias cannot take a different flock
    // or clobber the symlink on save.
    use intend::lockfile::{resolve_lockfile_path, Lockfile};
    use intend::store::{lockfile_lock_path, ScopeLock};
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let real = base.join("real-lock.json");
    Lockfile::load(&real).unwrap().save(&real).unwrap();
    let alias = base.join("alias-lock.json");
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    // Both names resolve to ONE identity…
    let r1 = resolve_lockfile_path(&real).unwrap();
    let r2 = resolve_lockfile_path(&alias).unwrap();
    assert_eq!(r1, r2);
    // …so the companion locks are the same lock: holding it via one name
    // blocks acquisition via the other.
    let held = ScopeLock::acquire(&lockfile_lock_path(&r1)).unwrap();
    assert!(ScopeLock::try_acquire(&lockfile_lock_path(&r2))
        .unwrap()
        .is_none());
    drop(held);

    // A save through the RESOLVED identity updates the target; the alias
    // stays a symlink pointing at it (never replaced by a regular file).
    let lock = Lockfile::load(&r2).unwrap();
    lock.save(&r2).unwrap();
    assert!(std::fs::symlink_metadata(&alias)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::symlink_metadata(&real).unwrap().is_file());

    // Dangling symlink: rejected. Directory: rejected. New file: parent
    // canonicalized + final name joined.
    let dangling = base.join("dangling.json");
    std::os::unix::fs::symlink(base.join("nope.json"), &dangling).unwrap();
    let err = format!("{:#}", resolve_lockfile_path(&dangling).unwrap_err());
    assert!(err.contains("does not resolve"), "{err}");
    let err = format!("{:#}", resolve_lockfile_path(&base).unwrap_err());
    assert!(err.contains("directory"), "{err}");
    std::fs::create_dir(base.join("sub")).unwrap();
    let fresh = resolve_lockfile_path(&base.join("sub").join("..").join("new.json"));
    assert_eq!(fresh.unwrap(), base.join("new.json"));
}

#[test]
fn reserved_lock_suffix_uses_the_filesystem_folding_model() {
    // Round-7: the ".lock" reservation must fold the way APFS folds — case-
    // and Unicode-insensitively — and must check the REQUESTED name before
    // any resolution, so no alias form slips into the companion namespace.
    use intend::lockfile::resolve_lockfile_path;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();

    // Case variants (checked on the requested name — no file needed).
    for name in ["x.LOCK", "x.LoCk", "x.lock"] {
        let err = format!("{:#}", resolve_lockfile_path(&base.join(name)).unwrap_err());
        assert!(err.contains("reserved"), "{name}: {err}");
    }
    // APFS Kelvin-sign alias: ".locK" with U+212A folds to ".lock".
    let kelvin = format!("x.loc{}", '\u{212A}');
    let err = format!(
        "{:#}",
        resolve_lockfile_path(&base.join(&kelvin)).unwrap_err()
    );
    assert!(err.contains("reserved"), "{err}");
    // Non-UTF-8 basenames are rejected outright (exact-path policy) — which
    // also closes the raw-bytes ".lock" bypass.
    use std::os::unix::ffi::OsStrExt;
    let raw = std::ffi::OsStr::from_bytes(b"bad\xff.lock");
    let err = format!("{:#}", resolve_lockfile_path(&base.join(raw)).unwrap_err());
    assert!(err.contains("UTF-8"), "{err}");
    // A REQUESTED reserved name must not slip through by resolving to an
    // innocent basename: symlink P.lock -> Q.json.
    let target = base.join("q.json");
    std::fs::write(&target, b"{}").unwrap();
    let alias = base.join("p.lock");
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let err = format!("{:#}", resolve_lockfile_path(&alias).unwrap_err());
    assert!(err.contains("reserved"), "{err}");
}

#[test]
fn scope_lock_sidecar_rejects_symlinks_fifos_and_foreign_files() {
    // Round-7: ScopeLock must never lock THROUGH a pre-existing node at the
    // sidecar path — a symlink there flocks a foreign data inode that an
    // atomic save then detaches (lock split); a FIFO could hang the open; a
    // non-empty regular file is somebody's data, not our sidecar.
    use intend::store::ScopeLock;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();

    // Symlink at the sidecar path (the P.lock -> Q.json attack): refused,
    // never followed.
    let data = base.join("q.json");
    std::fs::write(&data, b"{}").unwrap();
    let link_side = base.join("a.json.lock");
    std::os::unix::fs::symlink(&data, &link_side).unwrap();
    let err = format!("{:#}", ScopeLock::acquire(&link_side).unwrap_err());
    assert!(err.contains("SYMLINK"), "{err}");
    let err = format!("{:#}", ScopeLock::try_acquire(&link_side).unwrap_err());
    assert!(err.contains("SYMLINK"), "{err}");

    // FIFO at the sidecar path: typed refusal, no hang (O_NONBLOCK + fstat).
    let fifo_side = base.join("b.json.lock");
    let c = std::ffi::CString::new(fifo_side.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let err = format!("{:#}", ScopeLock::acquire(&fifo_side).unwrap_err());
    assert!(err.contains("dedicated empty regular"), "{err}");

    // Non-empty regular file at the sidecar path: somebody's data — refused.
    let full_side = base.join("c.json.lock");
    std::fs::write(&full_side, b"not a sidecar").unwrap();
    let err = format!("{:#}", ScopeLock::acquire(&full_side).unwrap_err());
    assert!(err.contains("dedicated empty regular"), "{err}");

    // A clean sidecar still works end to end, exclusivity included.
    let good = base.join("d.json.lock");
    let held = ScopeLock::acquire(&good).unwrap();
    assert!(ScopeLock::try_acquire(&good).unwrap().is_none());
    drop(held);
    assert!(ScopeLock::try_acquire(&good).unwrap().is_some());
}

#[test]
fn pre_context_lockfiles_and_catalogs_fail_closed() {
    // Round-7 regression pins: legacy state written BEFORE the deployment-
    // context binding must be rejected, never silently reinterpreted.
    use intend::lockfile::Lockfile;
    use intend::store::Catalog;
    let tmp = tempfile::tempdir().unwrap();

    // A version-1 lockfile (pre-context entries).
    let lock_path = tmp.path().join("old-lock.json");
    std::fs::write(&lock_path, br#"{"version":1,"entries":[]}"#).unwrap();
    let err = format!("{:#}", Lockfile::load(&lock_path).unwrap_err());
    assert!(err.contains("version 1"), "{err}");

    // A catalog meta without deploymentContextId (pre-context catalog): the
    // parse itself fails closed with the run-update guidance implied.
    std::fs::write(
        tmp.path().join("catalog-meta.json"),
        br#"{"verifiedAtUnix":1,"anchorBlock":1,"anchorBlockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","anchorStateRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","anchorMode":"t","items":0,"snapshotSha256":"00"}"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("catalog-snapshot.json"), b"{}").unwrap();
    let err = format!("{:#}", Catalog::load(tmp.path()).unwrap_err());
    assert!(
        err.contains("deploymentContextId") || err.contains("parsing"),
        "{err}"
    );
}

#[test]
fn innocent_symlink_resolving_to_a_reserved_or_non_utf8_target_rejects() {
    // Round-8 pin: the RESOLVED-basename branch of the reservation. An
    // innocently named lockfile path that is a symlink to a reserved target
    // must reject on the resolved name (fold included — the on-disk target
    // here is "q.LOCK").
    use intend::lockfile::resolve_lockfile_path;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let target = base.join("q.LOCK");
    std::fs::write(&target, b"").unwrap();
    let innocent = base.join("innocent.json");
    std::os::unix::fs::symlink(&target, &innocent).unwrap();
    let err = format!("{:#}", resolve_lockfile_path(&innocent).unwrap_err());
    assert!(err.contains("reserved"), "{err}");

    // Non-UTF-8 RESOLVED target, where the filesystem permits creating one
    // (APFS enforces UTF-8 names, so this leg self-skips on macOS).
    use std::os::unix::ffi::OsStrExt;
    let raw_target = base.join(std::ffi::OsStr::from_bytes(b"bad\xfftarget"));
    if std::fs::write(&raw_target, b"").is_ok() {
        let innocent2 = base.join("innocent2.json");
        std::os::unix::fs::symlink(&raw_target, &innocent2).unwrap();
        let err = format!("{:#}", resolve_lockfile_path(&innocent2).unwrap_err());
        assert!(err.contains("UTF-8"), "{err}");
    }
}

#[test]
fn hard_linked_sidecar_rejects_in_both_acquire_paths() {
    // Round-8 pin: an EMPTY hard-linked sidecar (nlink > 1) is refused by
    // BOTH ScopeLock entry points — a second name for the sidecar inode would
    // let an atomic replacement detach one path's view of the lock.
    use intend::store::ScopeLock;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let sidecar = base.join("s.json.lock");
    std::fs::write(&sidecar, b"").unwrap();
    std::fs::hard_link(&sidecar, base.join("second-name")).unwrap();
    let err = format!("{:#}", ScopeLock::acquire(&sidecar).unwrap_err());
    assert!(err.contains("dedicated empty regular"), "{err}");
    let err = format!("{:#}", ScopeLock::try_acquire(&sidecar).unwrap_err());
    assert!(err.contains("dedicated empty regular"), "{err}");
}

#[test]
fn oversized_account_proof_rejects_at_the_wire_boundary() {
    // PR #3 re-review: a "0x" account-proof element costs ~4 JSON bytes but
    // tens of heap bytes, so a document under the decoded cap could allocate
    // gigabytes BEFORE verify's count checks run. The deserializer itself now
    // bounds the collection, so the reject happens during parsing without
    // materializing the flood.
    let f = build_fixture();
    let mut v = serde_json::to_value(&f.snapshot).unwrap();
    v["proofs"]["account"] =
        serde_json::Value::Array(vec![serde_json::Value::String("0x".into()); 500]);
    let raw = serde_json::to_vec(&v).unwrap();
    let err = format!(
        "{:#}",
        intend::snapshot::read_snapshot_bounded(&raw, &Limits::default()).unwrap_err()
    );
    assert!(err.contains("account proof exceeds"), "{err}");
    // The honest fixture still round-trips (control).
    let raw = serde_json::to_vec(&f.snapshot).unwrap();
    intend::snapshot::read_snapshot_bounded(&raw, &Limits::default()).unwrap();
}
