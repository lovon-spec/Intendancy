//! JSON-RPC HTTP transport with a RESPONSE-BODY byte cap (review finding: a
//! deadline does not bound a fast oversized response). Every provider in this
//! crate — anchor sources and proof RPCs alike — is built through
//! `capped_provider`. The body is consumed through the same BLOCKING
//! take-bounded reader as `fetch`, after a content-length precheck: at most
//! cap + 1 bytes ever reach the application buffer. The same honest scope as
//! `fetch` applies (rounds 5-7): the locked HTTP stack below the reader
//! (reqwest 0.12.28 / hyper 1.11.0 / h2 0.4.18 per Cargo.lock) buffers
//! response bytes ahead of consumption within its configured PER-CONNECTION /
//! PER-STREAM bounds — the HTTP/1 read buffer (≤ 417,792 bytes) or the
//! explicitly configured HTTP/2 windows (see `fetch::http_client`) — plus one
//! adapter `Bytes` chunk and platform-managed TLS/socket buffers. Aggregate
//! memory scales with connections and caller concurrency (this type enforces
//! no process-wide cap); each component is independent of response size.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::client::RpcClient;
use alloy::transports::{TransportError, TransportErrorKind, TransportFut};
use eyre::Result;
use serde::{Deserialize, Serialize};

/// Cap on any single JSON-RPC response body. Generous versus every legitimate
/// response this crate makes (the largest are chunked `eth_getProof` batches at
/// low single-digit MB), tight versus memory exhaustion.
pub const RPC_RESPONSE_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// What the transport actually did, counted where it happens: every
/// JSON-RPC request handed to `call`, every HTTP attempt made for it
/// (retries included), and every response byte read. Shared by every
/// provider built with `capped_provider_counted`, so a run's provenance
/// reports measured work instead of a maintained multiplier.
#[derive(Debug, Default)]
pub struct RpcStats {
    requests: AtomicU64,
    attempts: AtomicU64,
    response_bytes: AtomicU64,
}

/// A point-in-time reading of `RpcStats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcCounts {
    /// JSON-RPC requests issued (one per provider call, batches counted once).
    pub requests: u64,
    /// HTTP attempts made for them, retries included.
    pub attempts: u64,
    /// Attempts beyond the first for a request (`attempts - requests`).
    pub retries: u64,
    /// Response body bytes read, over-cap reads included.
    #[serde(rename = "responseBytes")]
    pub response_bytes: u64,
}

impl RpcStats {
    pub fn counts(&self) -> RpcCounts {
        let requests = self.requests.load(Ordering::Relaxed);
        let attempts = self.attempts.load(Ordering::Relaxed);
        RpcCounts {
            requests,
            attempts,
            retries: attempts.saturating_sub(requests),
            response_bytes: self.response_bytes.load(Ordering::Relaxed),
        }
    }
}

impl RpcCounts {
    /// The work between two readings of the same counters.
    pub fn since(self, earlier: RpcCounts) -> RpcCounts {
        RpcCounts {
            requests: self.requests.saturating_sub(earlier.requests),
            attempts: self.attempts.saturating_sub(earlier.attempts),
            retries: self.retries.saturating_sub(earlier.retries),
            response_bytes: self.response_bytes.saturating_sub(earlier.response_bytes),
        }
    }
}

#[derive(Clone)]
pub struct CappedHttp {
    url: reqwest::Url,
    cap: u64,
    stats: Arc<RpcStats>,
}

/// Attempts made for one JSON-RPC request before its transient failure is
/// reported. Every request this crate makes is an idempotent read (chain id,
/// headers, proofs, `eth_call`), so a retry can never double an effect; public
/// endpoints answer 408/429/5xx or drop a connection often enough (dRPC,
/// observed 2026-09-07) that one such answer must not fail a whole run.
pub const RPC_ATTEMPTS: u32 = 3;

fn transient_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn rpc_blocking(
    url: reqwest::Url,
    cap: u64,
    body: Vec<u8>,
    stats: &RpcStats,
) -> Result<Vec<u8>, TransportError> {
    let mut attempt = 1;
    loop {
        stats.attempts.fetch_add(1, Ordering::Relaxed);
        match rpc_blocking_once(&url, cap, &body, stats) {
            Ok(buf) => return Ok(buf),
            Err((true, e)) if attempt < RPC_ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempt)));
                attempt += 1;
                let _ = e;
            }
            Err((_, e)) => return Err(e),
        }
    }
}

/// One attempt. The boolean says whether the failure is transient (worth a
/// retry): a failed send or a transient status. Cap violations and non-transient
/// statuses are final.
fn rpc_blocking_once(
    url: &reqwest::Url,
    cap: u64,
    body: &[u8],
    stats: &RpcStats,
) -> Result<Vec<u8>, (bool, TransportError)> {
    let client = crate::fetch::http_client()
        .map_err(|e| (false, TransportErrorKind::custom_str(&format!("{e:#}"))))?;
    let resp = client
        .post(url.clone())
        .timeout(std::time::Duration::from_secs(
            crate::anchor::RPC_DEADLINE_SECS,
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .send()
        .map_err(|e| (true, TransportErrorKind::custom(e)))?;
    let status = resp.status();
    if !status.is_success() {
        return Err((
            transient_status(status),
            TransportErrorKind::custom_str(&format!("HTTP {status} from {url}")),
        ));
    }
    if let Some(len) = resp.content_length() {
        if len > cap {
            return Err((
                false,
                TransportErrorKind::custom_str(&format!(
                    "RPC response advertises {len} bytes, over the {cap}-byte cap"
                )),
            ));
        }
    }
    // Take-bounded read: at most cap + 1 bytes are ever consumed (saturating).
    use std::io::Read;
    let mut buf = Vec::new();
    let read = resp
        .take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| {
            (
                true,
                TransportErrorKind::custom_str(&format!("reading RPC response: {e}")),
            )
        });
    stats
        .response_bytes
        .fetch_add(buf.len() as u64, Ordering::Relaxed);
    read?;
    if buf.len() as u64 > cap {
        return Err((
            false,
            TransportErrorKind::custom_str(&format!("RPC response exceeds the {cap}-byte cap")),
        ));
    }
    Ok(buf)
}

impl tower::Service<alloy::rpc::json_rpc::RequestPacket> for CappedHttp {
    type Response = alloy::rpc::json_rpc::ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: alloy::rpc::json_rpc::RequestPacket) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let body = serde_json::to_vec(&req).map_err(TransportError::ser_err)?;
            this.stats.requests.fetch_add(1, Ordering::Relaxed);
            let buf = tokio::task::spawn_blocking(move || {
                rpc_blocking(this.url, this.cap, body, &this.stats)
            })
            .await
            .map_err(|e| TransportErrorKind::custom_str(&format!("RPC task join: {e}")))??;
            serde_json::from_slice(&buf)
                .map_err(|e| TransportError::deser_err(e, String::from_utf8_lossy(&buf)))
        })
    }
}

/// Build a provider whose HTTP transport enforces `RPC_RESPONSE_CAP_BYTES` on
/// every response body, with a per-request deadline at the reqwest layer too.
pub fn capped_provider(rpc_url: &str) -> Result<impl Provider + Clone> {
    capped_provider_with_cap(rpc_url, RPC_RESPONSE_CAP_BYTES)
}

/// `capped_provider` whose requests, attempts and bytes are counted into
/// `stats` (shared across every provider of a run).
pub fn capped_provider_counted(
    rpc_url: &str,
    stats: Arc<RpcStats>,
) -> Result<impl Provider + Clone> {
    build_provider(rpc_url, RPC_RESPONSE_CAP_BYTES, stats)
}

/// As `capped_provider` with an explicit cap — production callers use the
/// default; tests inject tiny caps to exercise the oversized-response paths.
pub fn capped_provider_with_cap(rpc_url: &str, cap: u64) -> Result<impl Provider + Clone> {
    build_provider(rpc_url, cap, Arc::default())
}

fn build_provider(rpc_url: &str, cap: u64, stats: Arc<RpcStats>) -> Result<impl Provider + Clone> {
    let url: reqwest::Url = rpc_url.parse()?;
    let transport = CappedHttp { url, cap, stats };
    let rpc_client = RpcClient::new(transport, false);
    Ok(ProviderBuilder::new().connect_client(rpc_client))
}
