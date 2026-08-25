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

use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::client::RpcClient;
use alloy::transports::{TransportError, TransportErrorKind, TransportFut};
use eyre::Result;

/// Cap on any single JSON-RPC response body. Generous versus every legitimate
/// response this crate makes (the largest are chunked `eth_getProof` batches at
/// low single-digit MB), tight versus memory exhaustion.
pub const RPC_RESPONSE_CAP_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct CappedHttp {
    url: reqwest::Url,
    cap: u64,
}

fn rpc_blocking(url: reqwest::Url, cap: u64, body: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    let client = crate::fetch::http_client()
        .map_err(|e| TransportErrorKind::custom_str(&format!("{e:#}")))?;
    let resp = client
        .post(url.clone())
        .timeout(std::time::Duration::from_secs(
            crate::anchor::RPC_DEADLINE_SECS,
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .map_err(TransportErrorKind::custom)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(TransportErrorKind::custom_str(&format!(
            "HTTP {status} from {url}"
        )));
    }
    if let Some(len) = resp.content_length() {
        if len > cap {
            return Err(TransportErrorKind::custom_str(&format!(
                "RPC response advertises {len} bytes, over the {cap}-byte cap"
            )));
        }
    }
    // Take-bounded read: at most cap + 1 bytes are ever consumed (saturating).
    use std::io::Read;
    let mut buf = Vec::new();
    resp.take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| TransportErrorKind::custom_str(&format!("reading RPC response: {e}")))?;
    if buf.len() as u64 > cap {
        return Err(TransportErrorKind::custom_str(&format!(
            "RPC response exceeds the {cap}-byte cap"
        )));
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
            let buf = tokio::task::spawn_blocking(move || rpc_blocking(this.url, this.cap, body))
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

/// As `capped_provider` with an explicit cap — production callers use the
/// default; tests inject tiny caps to exercise the oversized-response paths.
pub fn capped_provider_with_cap(rpc_url: &str, cap: u64) -> Result<impl Provider + Clone> {
    let url: reqwest::Url = rpc_url.parse()?;
    let transport = CappedHttp { url, cap };
    let rpc_client = RpcClient::new(transport, false);
    Ok(ProviderBuilder::new().connect_client(rpc_client))
}
