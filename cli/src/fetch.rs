//! Bounded HTTP fetching of untrusted bytes (CARs from IPFS gateways, snapshots
//! from provider URLs), through a BLOCKING take-bounded reader implementing
//! the §4.1 application-layer contract: at most cap + 1 bytes are ever
//! consumed into the application's buffer, regardless of the source's size
//! (`io::Read` + `Read::take`; the counting-reader tests below prove the
//! reader-layer consumption bound). Honest scope (rounds 5-7): BELOW this
//! reader, the locked HTTP stack (Cargo.lock: reqwest 0.12.28, hyper 1.11.0,
//! h2 0.4.18) buffers response bytes ahead of consumption in its own
//! bounded-by-configuration buffers — HTTP/1: hyper's read buffer, ≤ 417,792
//! bytes (`DEFAULT_MAX_BUFFER_SIZE`); HTTP/2: the flow-control windows this
//! crate configures explicitly on the client (stream 1 MiB, connection
//! 2 MiB, frame 16 KiB), under which multiple DATA frames may be queued.
//! These are PER-CONNECTION / PER-STREAM component bounds, not a process
//! ceiling: aggregate memory scales with live and idle-pooled connections
//! and caller concurrency (the CLI's command paths are sequential; the
//! transport type enforces no process-wide cap). The blocking-reader adapter
//! retains one in-flight `Bytes` chunk; TLS and kernel socket buffers are
//! platform-managed and not quantified here. All of it is
//! response-size-INDEPENDENT overhead that `Read::take` does not and cannot
//! eliminate. All cap arithmetic is saturating. Sources are never trusted;
//! everything is verified downstream. FAILOVER is the caller's job and must
//! continue past verification failures, not just HTTP failures — these
//! helpers fetch from ONE source.

use std::io::Read;

use eyre::{bail, Context, Result};

/// Shared blocking HTTP client. Built lazily INSIDE a blocking context and
/// held in a static (a `reqwest::blocking::Client` must not be created or
/// dropped on an async runtime thread; a static is never dropped). No default
/// timeout — every request sets its own. HTTP/2 flow-control is EXPLICITLY
/// configured (round-6) so the below-reader buffering bound is a value THIS
/// crate states, not an upstream default: stream window 1 MiB, connection
/// window 2 MiB, max frame 16 KiB.
pub(crate) const H2_STREAM_WINDOW_BYTES: u32 = 1 << 20;
pub(crate) const H2_CONNECTION_WINDOW_BYTES: u32 = 2 << 20;
pub(crate) const H2_MAX_FRAME_BYTES: u32 = 1 << 14;

pub(crate) fn http_client() -> Result<&'static reqwest::blocking::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if CLIENT.get().is_none() {
        let client = reqwest::blocking::Client::builder()
            .http2_initial_stream_window_size(H2_STREAM_WINDOW_BYTES)
            .http2_initial_connection_window_size(H2_CONNECTION_WINDOW_BYTES)
            .http2_max_frame_size(H2_MAX_FRAME_BYTES)
            .build()
            .wrap_err("building HTTP client")?;
        let _ = CLIENT.set(client);
    }
    Ok(CLIENT.get().expect("just initialized"))
}

const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Read at most cap + 1 bytes (saturating) from `r`; error if more than `cap`
/// arrived. The take-style bound means an oversized source is abandoned after
/// cap + 1 bytes are CONSUMED from the reader — nothing beyond that reaches
/// the application buffer (§4.1's application-layer bound; the module doc
/// states the pinned transport-frame overhead below the reader). Unit tests
/// prove the consumption bound with a counting reader.
pub fn read_bounded(r: impl Read, cap: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut limited = r.take(cap.saturating_add(1));
    limited
        .read_to_end(&mut buf)
        .wrap_err("reading bounded body")?;
    if buf.len() as u64 > cap {
        bail!("response exceeds the {cap}-byte cap");
    }
    Ok(buf)
}

/// Snapshot body reader implementing §4.1's sniff-then-exact-cap contract over
/// a raw `io::Read`: first read the 2 gzip-magic sniff bytes (themselves
/// bounded by the LARGER cap + 1, so degenerate caps still bound the very
/// first read), select the applicable cap from the magic, reject already-read
/// bytes that exceed it, then continue under a take bound of exactly
/// cap + 1 TOTAL bytes — the sniff bytes count toward the total, so at most
/// cap + 1 bytes are ever consumed from the reader for any cap ≥ 1 (and at
/// most cap_max + 1 in the degenerate sub-2-byte-cap corner).
pub fn read_snapshot_body(mut r: impl Read, limits: &crate::snapshot::Limits) -> Result<Vec<u8>> {
    let cap_max = limits.max_compressed_bytes.max(limits.max_decoded_bytes);
    let sniff_len = 2u64.min(cap_max.saturating_add(1)) as usize;
    let mut head = [0u8; 2];
    let mut got = 0usize;
    while got < sniff_len {
        let n = r
            .read(&mut head[got..sniff_len])
            .wrap_err("reading snapshot body")?;
        if n == 0 {
            break;
        }
        got += n;
    }
    // A sub-2-byte body cannot be gzip; the raw (decoded) cap applies.
    let cap = if got == 2 && head == [0x1f, 0x8b] {
        limits.max_compressed_bytes
    } else {
        limits.max_decoded_bytes
    };
    if got as u64 > cap {
        bail!("response exceeds the {cap}-byte cap for its encoding");
    }
    let mut buf = Vec::from(&head[..got]);
    let remaining = cap.saturating_add(1).saturating_sub(got as u64);
    let mut limited = r.take(remaining);
    limited
        .read_to_end(&mut buf)
        .wrap_err("reading snapshot body")?;
    if buf.len() as u64 > cap {
        bail!("response exceeds the {cap}-byte cap for its encoding");
    }
    Ok(buf)
}

/// GET `url` with the response body capped at `cap` bytes: advertised
/// content-length is prechecked, and the body is then consumed through the
/// take-bounded reader (at most cap + 1 bytes read).
pub async fn fetch_bounded(url: &str, cap: u64, accept: Option<&str>) -> Result<Vec<u8>> {
    let url = url.to_string();
    let accept = accept.map(str::to_owned);
    tokio::task::spawn_blocking(move || fetch_bounded_blocking(&url, cap, accept.as_deref()))
        .await
        .wrap_err("fetch task")?
}

fn fetch_bounded_blocking(url: &str, cap: u64, accept: Option<&str>) -> Result<Vec<u8>> {
    let mut req = http_client()?.get(url).timeout(FETCH_TIMEOUT);
    if let Some(a) = accept {
        req = req.header("Accept", a);
    }
    let resp = req.send()?.error_for_status()?;
    if let Some(len) = resp.content_length() {
        if len > cap {
            bail!("{url}: advertised {len} bytes exceeds the {cap}-byte cap");
        }
    }
    read_bounded(resp, cap).wrap_err_with(|| format!("{url}: over-cap or read failure"))
}

/// Fetch ONE snapshot candidate with the §4.1 cap contract applied at the
/// read boundary: gzip input is capped at `limits.max_compressed_bytes` and
/// raw input at `limits.max_decoded_bytes`, with at most cap + 1 bytes ever
/// read (see `read_snapshot_body`). The caller then runs the bounded parse.
pub async fn fetch_snapshot(url: &str, limits: &crate::snapshot::Limits) -> Result<Vec<u8>> {
    let url = url.to_string();
    let limits = limits.clone();
    tokio::task::spawn_blocking(move || fetch_snapshot_blocking(&url, &limits))
        .await
        .wrap_err("fetch task")?
}

fn fetch_snapshot_blocking(url: &str, limits: &crate::snapshot::Limits) -> Result<Vec<u8>> {
    let resp = http_client()?
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()?
        .error_for_status()?;
    if let Some(len) = resp.content_length() {
        let cap_max = limits.max_compressed_bytes.max(limits.max_decoded_bytes);
        if len > cap_max {
            bail!("{url}: advertised {len} bytes exceeds every applicable cap ({cap_max})");
        }
    }
    read_snapshot_body(resp, limits).wrap_err_with(|| format!("{url}: over-cap or read failure"))
}

/// Fetch a CAR for `cid` from one trustless gateway.
pub async fn fetch_car(gateway: &str, cid: &str) -> Result<Vec<u8>> {
    let url = format!("{}/ipfs/{cid}?format=car", gateway.trim_end_matches('/'));
    fetch_bounded(
        &url,
        crate::car::MAX_CAR_BYTES,
        Some("application/vnd.ipld.car"),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{read_bounded, read_snapshot_body};
    use crate::snapshot::Limits;
    use std::io::Read;

    /// Infinite reader that COUNTS every byte handed out — the §4.1
    /// pre-materialization proof: however large the source, consumption stops
    /// at the take bound.
    struct Counting {
        byte: u8,
        first_two: [u8; 2],
        pos: u64,
        consumed: std::rc::Rc<std::cell::Cell<u64>>,
    }

    impl Counting {
        fn new(first_two: [u8; 2], fill: u8) -> (Self, std::rc::Rc<std::cell::Cell<u64>>) {
            let consumed = std::rc::Rc::new(std::cell::Cell::new(0));
            (
                Self {
                    byte: fill,
                    first_two,
                    pos: 0,
                    consumed: consumed.clone(),
                },
                consumed,
            )
        }
    }

    impl Read for Counting {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            for slot in out.iter_mut() {
                *slot = match self.pos {
                    0 => self.first_two[0],
                    1 => self.first_two[1],
                    _ => self.byte,
                };
                self.pos += 1;
            }
            self.consumed.set(self.consumed.get() + out.len() as u64);
            Ok(out.len())
        }
    }

    fn limits(compressed: u64, decoded: u64) -> Limits {
        Limits {
            max_compressed_bytes: compressed,
            max_decoded_bytes: decoded,
            ..Limits::default()
        }
    }

    #[test]
    fn gzip_source_consumption_stops_at_compressed_cap_plus_one() {
        let (r, consumed) = Counting::new([0x1f, 0x8b], 0);
        let err = read_snapshot_body(r, &limits(1024, 1 << 30)).unwrap_err();
        assert!(format!("{err:#}").contains("cap"), "{err:#}");
        assert_eq!(
            consumed.get(),
            1025,
            "an infinite gzip source must be abandoned after exactly cap + 1 bytes"
        );
    }

    #[test]
    fn raw_source_consumption_stops_at_decoded_cap_plus_one() {
        let (r, consumed) = Counting::new(*b"{\"", b' ');
        let err = read_snapshot_body(r, &limits(1 << 30, 4096)).unwrap_err();
        assert!(format!("{err:#}").contains("cap"), "{err:#}");
        assert_eq!(consumed.get(), 4097);
    }

    #[test]
    fn degenerate_caps_bound_even_the_sniff_read() {
        // Both caps zero: at most max_cap + 1 = 1 byte is ever read.
        let (r, consumed) = Counting::new(*b"xx", b'x');
        let err = read_snapshot_body(r, &limits(0, 0)).unwrap_err();
        assert!(format!("{err:#}").contains("cap"), "{err:#}");
        assert_eq!(consumed.get(), 1);

        // Tiny asymmetric caps: gzip body, compressed cap 0 — the 2 sniff
        // bytes (bounded by max_cap + 1) already exceed it.
        let (r, consumed) = Counting::new([0x1f, 0x8b], 0);
        let err = read_snapshot_body(r, &limits(0, 8)).unwrap_err();
        assert!(format!("{err:#}").contains("cap"), "{err:#}");
        assert!(consumed.get() <= 2);
    }

    #[test]
    fn read_bounded_consumption_stops_at_cap_plus_one() {
        let (r, consumed) = Counting::new([0, 0], 0);
        let err = read_bounded(r, 512).unwrap_err();
        assert!(format!("{err:#}").contains("cap"), "{err:#}");
        assert_eq!(consumed.get(), 513);
    }

    #[test]
    fn under_cap_bodies_pass_through_exactly() {
        let body = b"{\"snapshotVersion\":\"0.2\"}".to_vec();
        let out =
            read_snapshot_body(std::io::Cursor::new(body.clone()), &limits(16, 4096)).unwrap();
        assert_eq!(out, body);
        let tiny =
            read_snapshot_body(std::io::Cursor::new(b"x".to_vec()), &limits(0, 4096)).unwrap();
        assert_eq!(tiny, b"x");
        let empty = read_snapshot_body(std::io::Cursor::new(Vec::new()), &limits(0, 0)).unwrap();
        assert!(empty.is_empty());
        let exact = read_bounded(std::io::Cursor::new(vec![7u8; 64]), 64).unwrap();
        assert_eq!(exact.len(), 64);
    }
}
