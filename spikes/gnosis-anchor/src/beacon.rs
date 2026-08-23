//! Untrusted data sources: Beacon API (light-client endpoints) and execution JSON-RPC
//! (`eth_getProof`), with capture-to-fixture and offline-replay modes.
//!
//! Everything fetched here is UNTRUSTED INPUT. Nothing in a response is believed until
//! it passes verification in `consensus.rs` / `proof.rs`. Responses are size-bounded.
//! Live capture writes the raw bodies so the default (offline) test suite replays the
//! exact bytes without any endpoint trust or availability.

use std::fs;
use std::path::{Path, PathBuf};

use alloy::primitives::{Address, B256};
use alloy::rpc::types::EIP1186AccountProofResponse;
use eyre::{bail, eyre, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::atomic_file::replace_atomically;
use crate::gnosis::{GnosisConsensusSpec, GENESIS_TIME, GENESIS_VALIDATORS_ROOT};
use helios_consensus_core::types::{Bootstrap, FinalityUpdate, Update};

/// Light-client payload versions this spike understands. Anything else — including a
/// future fork like "gloas" — fails closed at decode time rather than being guessed at.
pub const ALLOWED_LC_VERSIONS: &[&str] = &["capella", "deneb", "electra", "fulu"];

/// 16 MiB response cap: bounded parsing for untrusted endpoints.
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

/// Atomically replace one captured fixture without ever opening the destination for
/// writing. This matters even for a non-production spike: a pre-existing symlink or
/// hardlink at the fixture path must not let response capture mutate the high-water
/// file (or any other file) before verification fails closed.
pub(crate) fn write_capture_file(dir: &Path, name: &str, body: &[u8]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let destination = dir.join(name);
    replace_atomically(&destination, body, "capture")
}

pub enum Source {
    Live {
        consensus_url: String,
        execution_url: String,
        capture_dir: Option<PathBuf>,
    },
    Offline(PathBuf),
}

impl Source {
    fn http_get(url: &str) -> Result<Vec<u8>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client
            .get(url)
            .header("accept", "application/json")
            .send()
            .wrap_err_with(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = read_bounded(resp)?;
        if !status.is_success() {
            bail!(
                "GET {url}: HTTP {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        Ok(body)
    }

    fn obtain(&self, fixture_name: &str, live_url: impl FnOnce(&str) -> String) -> Result<Vec<u8>> {
        match self {
            Source::Live {
                consensus_url,
                capture_dir,
                ..
            } => {
                let body = Self::http_get(&live_url(consensus_url))?;
                if let Some(dir) = capture_dir {
                    write_capture_file(dir, fixture_name, &body)?;
                }
                Ok(body)
            }
            Source::Offline(dir) => read_fixture(dir, fixture_name),
        }
    }

    /// GET /eth/v1/beacon/genesis, cross-checked against the locally pinned constants.
    /// A mismatch is fatal: the endpoint is not serving Gnosis mainnet.
    pub fn check_genesis(&self) -> Result<()> {
        let body = self.obtain("genesis.json", |base| {
            format!("{base}/eth/v1/beacon/genesis")
        })?;
        let v: Value = serde_json::from_slice(&body).wrap_err("parsing genesis response")?;
        let data = &v["data"];
        let time: u64 = data["genesis_time"]
            .as_str()
            .ok_or_else(|| eyre!("genesis_time missing"))?
            .parse()?;
        let root: B256 = data["genesis_validators_root"]
            .as_str()
            .ok_or_else(|| eyre!("genesis_validators_root missing"))?
            .parse()?;
        if time != GENESIS_TIME || root != GENESIS_VALIDATORS_ROOT {
            return Err(crate::errors::StageError::GenesisMismatch {
                got_time: time,
                got_root: root,
                want_time: GENESIS_TIME,
                want_root: GENESIS_VALIDATORS_ROOT,
            }
            .into());
        }
        Ok(())
    }

    pub fn bootstrap(&self, checkpoint: B256) -> Result<Bootstrap<GnosisConsensusSpec>> {
        let body = self.obtain(&format!("bootstrap-{checkpoint}.json"), |base| {
            format!("{base}/eth/v1/beacon/light_client/bootstrap/{checkpoint}")
        })?;
        decode_versioned(&body).wrap_err("decoding light-client bootstrap")
    }

    pub fn updates(
        &self,
        start_period: u64,
        count: u64,
    ) -> Result<Vec<Update<GnosisConsensusSpec>>> {
        let body = self.obtain(
            &format!("updates-{start_period}-{count}.json"),
            |base| {
                format!(
                    "{base}/eth/v1/beacon/light_client/updates?start_period={start_period}&count={count}"
                )
            },
        )?;
        let mut entries: Vec<Value> =
            serde_json::from_slice(&body).wrap_err("parsing updates array")?;
        // Some servers (observed: rpc-gbc.gnosischain.com) append historical backfill
        // entries beyond the requested window. Only the requested window is decoded;
        // everything past `count` is discarded unexamined (never trusted, never fatal).
        entries.truncate(count as usize);
        entries
            .into_iter()
            .map(|entry| decode_versioned_value(entry).wrap_err("decoding light-client update"))
            .collect()
    }

    pub fn finality_update(&self) -> Result<FinalityUpdate<GnosisConsensusSpec>> {
        let body = self.obtain("finality_update.json", |base| {
            format!("{base}/eth/v1/beacon/light_client/finality_update")
        })?;
        decode_versioned(&body).wrap_err("decoding light-client finality update")
    }

    /// `eth_getProof` for the profile's address and locally derived slots, at the exact
    /// authenticated finalized execution block.
    pub fn get_proof(
        &self,
        address: Address,
        slots: &[B256],
        block_number: u64,
    ) -> Result<EIP1186AccountProofResponse> {
        let fixture = format!("proof-{block_number}.json");
        let body = match self {
            Source::Live {
                execution_url,
                capture_dir,
                ..
            } => {
                let params = serde_json::json!([address, slots, format!("0x{block_number:x}"),]);
                let req = serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "eth_getProof", "params": params,
                });
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?;
                let resp = client
                    .post(execution_url)
                    .json(&req)
                    .send()
                    .wrap_err_with(|| format!("POST eth_getProof to {execution_url}"))?;
                let body = read_bounded(resp)?;
                if let Some(dir) = capture_dir {
                    write_capture_file(dir, &fixture, &body)?;
                }
                body
            }
            Source::Offline(dir) => read_fixture(dir, &fixture)?,
        };
        let v: Value = serde_json::from_slice(&body).wrap_err("parsing eth_getProof response")?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            bail!("eth_getProof error from provider: {err}");
        }
        let result = v
            .get("result")
            .cloned()
            .ok_or_else(|| eyre!("eth_getProof response missing result"))?;
        serde_json::from_value(result).wrap_err("decoding EIP1186AccountProofResponse")
    }
}

fn read_bounded(resp: reqwest::blocking::Response) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut limited = resp.take(MAX_BODY_BYTES + 1);
    limited.read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_BODY_BYTES {
        bail!("response exceeds {MAX_BODY_BYTES} byte bound");
    }
    Ok(buf)
}

fn read_fixture(dir: &Path, name: &str) -> Result<Vec<u8>> {
    let path = dir.join(name);
    fs::read(&path).wrap_err_with(|| format!("reading fixture {}", path.display()))
}

fn decode_versioned<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    let v: Value = serde_json::from_slice(body).wrap_err("parsing versioned envelope")?;
    decode_versioned_value(v)
}

/// {version, data} envelope: the fork version is allow-listed BEFORE the payload is
/// decoded, so an unknown fork's schema is never guessed at (fails closed).
fn decode_versioned_value<T: DeserializeOwned>(v: Value) -> Result<T> {
    let version = v["version"]
        .as_str()
        .ok_or_else(|| eyre!("envelope missing fork version"))?
        .to_lowercase();
    if !ALLOWED_LC_VERSIONS.contains(&version.as_str()) {
        return Err(crate::errors::StageError::UnsupportedForkVersion {
            version,
            allowed: format!("{ALLOWED_LC_VERSIONS:?}"),
        }
        .into());
    }
    let data = v
        .get("data")
        .cloned()
        .ok_or_else(|| eyre!("envelope missing data"))?;
    serde_json::from_value(data).wrap_err_with(|| format!("decoding '{version}' payload"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn capture_replacement_does_not_follow_static_links() {
        let tmp = tempfile::tempdir().unwrap();
        let capture = tmp.path().join("capture");
        fs::create_dir_all(&capture).unwrap();

        // Model the earliest response capture and stop immediately afterward: even if
        // later verification/load fails, a hardlinked high-water file stays untouched.
        let state = tmp.path().join("highwater.json");
        let state_body = br#"{"version":1,"marks":{}}"#;
        fs::write(&state, state_body).unwrap();
        fs::hard_link(&state, capture.join("genesis.json")).unwrap();
        write_capture_file(&capture, "genesis.json", b"untrusted genesis response").unwrap();
        assert_eq!(fs::read(&state).unwrap(), state_body);
        assert_eq!(
            fs::read(capture.join("genesis.json")).unwrap(),
            b"untrusted genesis response"
        );

        // A destination symlink is replaced as a directory entry, not followed.
        let outside = tmp.path().join("outside.json");
        fs::write(&outside, b"outside must survive").unwrap();
        let linked_fixture = capture.join("finality_update.json");
        std::os::unix::fs::symlink(&outside, &linked_fixture).unwrap();
        write_capture_file(
            &capture,
            "finality_update.json",
            b"untrusted finality response",
        )
        .unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"outside must survive");
        assert_eq!(
            fs::read(&linked_fixture).unwrap(),
            b"untrusted finality response"
        );
        assert!(!fs::symlink_metadata(linked_fixture)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
