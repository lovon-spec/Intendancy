//! CLI-surface regressions (round 3): flags that must be refused syntactically.

use std::process::Command;

#[test]
fn offline_dir_conflicts_with_capture_dir() {
    // Offline replay must never write over its own inputs: --capture-dir is live-only.
    let out = Command::new(env!("CARGO_BIN_EXE_gnosis-anchor-spike"))
        .args([
            "--checkpoint",
            "0x28120630451d1d5a842fd5e9b19c8b30bb9ea4928a8a06e8a1f50e0cbce0ea5f",
            "--offline-dir",
            "fixtures",
            "--capture-dir",
            "/tmp/does-not-matter",
        ])
        .output()
        .expect("spawn CLI");
    assert!(!out.status.success(), "offline + capture must be refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("conflicts"),
        "expected a clap conflict error, got: {stderr}"
    );
}
