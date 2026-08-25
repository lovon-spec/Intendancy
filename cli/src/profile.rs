//! The locally pinned registry profile (spec §3): the trust configuration that
//! never comes from a provider. Loaded from TOML supplied by the operator or a
//! pinned release.

use alloy::primitives::{Address, B256};
use eyre::{bail, Context, Result};
use serde::Deserialize;

use crate::anchor::QuorumAnchor;
use crate::snapshot::{VerifierProfile, SNAPSHOT_VERSION};

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Chain the registry lives on (Gnosis = 100). Authenticated against every
    /// anchor source and proof RPC via `eth_chainId` (spec §7).
    pub chain_id: u64,
    /// The chain's genesis block hash — pinned chain identity, checked against
    /// every anchor source and proof RPC before use (spec §7).
    pub genesis_hash: B256,
    /// The one registry deployment this profile trusts.
    pub registry: Address,
    /// keccak256 of the registry's deployed runtime code — binds the bytecode
    /// and therefore the storage layout at the pinned address (spec §3/§10).
    pub registry_code_hash: B256,
    /// Header-quorum sources (spec §7): ≥ 2 RPC endpoints, pairwise distinct
    /// after URL normalization, whose finalized headers must agree. Distinct
    /// URLs are necessary, not sufficient — operators SHOULD be distinct trust
    /// domains (declare them in `anchor_operators`).
    pub anchor_rpcs: Vec<String>,
    /// Optional operator/trust-domain labels, parallel to `anchor_rpcs`; when
    /// present they must be pairwise distinct.
    #[serde(default)]
    pub anchor_operators: Vec<String>,
    /// Snapshot provider URLs tried in order by `update` (untrusted; bounded
    /// fetch + full verification; failover continues past verification
    /// failures).
    #[serde(default)]
    pub snapshot_urls: Vec<String>,
    /// MANDATORY pins of the DEPLOYMENT MetaEvidence/policy references (spec §3).
    /// Trust model: this profile is a locally authenticated deployment manifest —
    /// it pins the registry, its codehash, and the two INITIAL MetaEvidence
    /// references the deployment events declared; the on-chain
    /// `metaEvidenceUpdates == 0` proof (spec §6 step 3b) then establishes that
    /// no later policy was ever declared, so these pinned references ARE the
    /// policy every verdict was judged under.
    pub registration_meta_evidence: String,
    pub clearing_meta_evidence: String,
    /// Optional RPC used as the untrusted snapshot/proof PROVIDER (self-generation
    /// and point-check proofs). Everything it serves is verified locally.
    pub provider_rpc: Option<String>,
    /// IPFS HTTP gateways tried in order for CAR retrieval (untrusted; every
    /// block is hash-verified).
    #[serde(default)]
    pub gateways: Vec<String>,
    /// TEST-ONLY: fork/dev chains (anvil fork mode) serve headers with a ZERO
    /// stateRoot; with this set, the state root is derived from a probe proof's
    /// root node instead (`keccak256(accountProof[0])` — the Gate 2 spike's
    /// documented workaround), which DEGRADES the anchor: the root comes from
    /// the untrusted provider, only block number/hash stay quorum-checked.
    /// Every command labels its output accordingly. NEVER set in production.
    #[serde(default)]
    pub test_headerless_state_root: bool,
}

impl Profile {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("reading profile {}", path.display()))?;
        let profile: Profile =
            toml::from_str(&raw).wrap_err_with(|| format!("parsing profile {}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        let mut origins: Vec<String> = Vec::new();
        for rpc in &self.anchor_rpcs {
            origins.push(normalized_origin(rpc)?);
        }
        let mut distinct = origins.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() < 2 || distinct.len() != origins.len() {
            bail!(
                "anchor_rpcs must be ≥2 endpoints with pairwise-DISTINCT normalized \
                 origins (scheme://host:port, loopback aliases folded); got origins {:?}",
                origins
            );
        }
        for (field, value) in [
            (
                "registration_meta_evidence",
                &self.registration_meta_evidence,
            ),
            ("clearing_meta_evidence", &self.clearing_meta_evidence),
        ] {
            validate_meta_evidence_ref(field, value)?;
        }
        if self.registration_meta_evidence == self.clearing_meta_evidence {
            bail!(
                "registration_meta_evidence and clearing_meta_evidence must differ \
                 (two distinct MetaEvidence documents are emitted at deployment — \
                 DeployRegistry enforces the same rule)"
            );
        }
        if !self.anchor_operators.is_empty() {
            if self.anchor_operators.len() != self.anchor_rpcs.len() {
                bail!(
                    "anchor_operators must be parallel to anchor_rpcs ({} vs {})",
                    self.anchor_operators.len(),
                    self.anchor_rpcs.len()
                );
            }
            let mut ops = self.anchor_operators.clone();
            ops.sort_unstable();
            ops.dedup();
            if ops.len() != self.anchor_operators.len() {
                bail!("anchor_operators must be pairwise distinct trust domains");
            }
        }
        Ok(())
    }

    /// Versioned, domain-separated id of the ENTIRE local trust context
    /// (round-6): keccak256 over a length-prefixed encoding of every profile
    /// field that defines WHICH deployment — and which policy/anchor-mode
    /// context — local state was verified under: chain_id, genesis_hash,
    /// registry, registry_code_hash, both MetaEvidence pins, and the
    /// test-workaround flag. Two profiles differing in ANY of these (a
    /// same-chain-id fork with another genesis, the same address under other
    /// bytecode, edited policy pins, test mode) produce different ids, so
    /// catalogs, lockfile entries, and enable proofs made under one context
    /// can never be consumed under another.
    pub fn deployment_context_id(&self) -> B256 {
        use alloy::primitives::keccak256;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"intendhub-deployment-context-v1");
        buf.extend_from_slice(&self.chain_id.to_be_bytes());
        buf.extend_from_slice(self.genesis_hash.as_slice());
        buf.extend_from_slice(self.registry.as_slice());
        buf.extend_from_slice(self.registry_code_hash.as_slice());
        for s in [
            &self.registration_meta_evidence,
            &self.clearing_meta_evidence,
        ] {
            buf.extend_from_slice(&(s.len() as u64).to_be_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        buf.push(self.test_headerless_state_root as u8);
        keccak256(&buf)
    }

    /// The full proof context handed to state transitions (`enable_transition`).
    pub fn proof_context(&self) -> crate::lockfile::ProofContext {
        crate::lockfile::ProofContext {
            chain_id: self.chain_id,
            registry: self.registry,
            context_id: self.deployment_context_id(),
        }
    }

    /// The RPC used for proof harvesting (untrusted provider role).
    pub fn proof_rpc(&self) -> &str {
        self.provider_rpc
            .as_deref()
            .unwrap_or_else(|| self.anchor_rpcs[0].as_str())
    }

    /// Assemble the §6 verifier profile from this pin plus an AUTHENTICATED
    /// anchor (the caller obtained it via a §7 mode).
    pub fn verifier_profile(&self, anchor: &QuorumAnchor) -> VerifierProfile {
        VerifierProfile {
            version: SNAPSHOT_VERSION.into(),
            chain_id: self.chain_id,
            registry: self.registry,
            registry_code_hash: self.registry_code_hash,
            anchor_block: anchor.block_number,
            anchor_block_hash: anchor.block_hash,
            anchor_state_root: anchor.state_root,
        }
    }
}

/// Canonicality of a pinned deployment MetaEvidence reference — a STRICT
/// SUPERSET of the rules `DeployRegistry.s.sol::_validateMetaEvidenceUri`
/// enforces at deployment: `/ipfs/` + a canonical CIDv1 (59 chars: `b` +
/// lowercase base32 `[a-z2-7]`, the one-byte-codec sha2-256 form) +
/// optionally a `/`-separated path with NO empty segments; no `TODO`
/// placeholder. Beyond the script's shape checks, this client requires the
/// CID to strictly DECODE (round-5; the Solidity check is alphabet-only) —
/// strengthening DeployRegistry to match is pre-production work. Trust boundary, stated exactly: the profile is
/// a locally authenticated deployment manifest — this check establishes the
/// pinned references are WELL-FORMED and distinct, and §6 step 3b's
/// `metaEvidenceUpdates == 0` proof establishes no LATER policy was ever
/// declared; that these are the references the deployment's MetaEvidence
/// events actually emitted is the profile author's release-manifest assertion
/// (enforced at deployment by the script, not re-derived from event logs by
/// this client).
fn validate_meta_evidence_ref(field: &str, value: &str) -> Result<()> {
    const PREFIX: &str = "/ipfs/";
    const CID_LEN: usize = 59;
    let b = value.as_bytes();
    if !value.starts_with(PREFIX) || b.len() < PREFIX.len() + CID_LEN {
        bail!(
            "{field} must be \"/ipfs/<canonical CIDv1>[/<path>]\" — the deployment \
             policy reference the counter proof makes authoritative (spec §3)"
        );
    }
    let cid = &b[PREFIX.len()..PREFIX.len() + CID_LEN];
    if cid[0] != b'b' {
        bail!("{field}: CID must be lowercase base32 CIDv1 ('b…')");
    }
    if !cid[1..]
        .iter()
        .all(|c| matches!(c, b'a'..=b'z' | b'2'..=b'7'))
    {
        bail!("{field}: CID is not canonical lowercase base32");
    }
    // Round-4: shape alone is not canonicality. The CID must STRICTLY decode —
    // canonical base32 (data_encoding rejects nonzero trailing bits), CIDv1,
    // sha2-256/32 multihash, raw/dag-pb codec — via the same strict parser the
    // installer trusts for Tree CIDs.
    let cid_str = std::str::from_utf8(cid).expect("alphabet-checked above");
    crate::car::Cid::parse_canonical(cid_str).map_err(|e| {
        eyre::eyre!(
            "{field}: CID does not strictly decode to canonical \
             CIDv1/sha2-256-32 (raw or dag-pb): {e:#}"
        )
    })?;
    if b.len() > PREFIX.len() + CID_LEN {
        if b[PREFIX.len() + CID_LEN] != b'/' {
            bail!("{field}: CID has an invalid suffix (only a '/'-separated path may follow)");
        }
        if b.len() == PREFIX.len() + CID_LEN + 1 {
            bail!("{field}: path after the CID must not be empty");
        }
        let path = &value[PREFIX.len() + CID_LEN + 1..];
        if path.split('/').any(|seg| seg.is_empty()) {
            bail!("{field}: path after the CID must not contain empty segments");
        }
    }
    if value.contains("TODO") {
        bail!("{field} must not contain a TODO placeholder");
    }
    Ok(())
}

/// Normalize an RPC URL to its origin for the distinctness rule (spec §7):
/// scheme + lowercased host (loopback aliases folded to 127.0.0.1) + effective
/// port. Path/query differences never make two endpoints distinct.
pub fn normalized_origin(raw: &str) -> Result<String> {
    let parsed = url::Url::parse(raw).wrap_err_with(|| format!("invalid RPC URL {raw:?}"))?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    let host = parsed
        .host_str()
        .ok_or_else(|| eyre::eyre!("RPC URL {raw:?} has no host"))?
        .to_ascii_lowercase();
    let host = match host.as_str() {
        "localhost" | "[::1]" | "::1" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| eyre::eyre!("RPC URL {raw:?} has no resolvable port"))?;
    Ok(format!("{scheme}://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::{normalized_origin, validate_meta_evidence_ref, Profile};

    fn base_profile() -> Profile {
        let cid = "bafybeidgtfsc2ro3pfmyggmbz4ea7xg7g4gpehqur7klaadtreyjz6s3fu";
        Profile {
            chain_id: 100,
            genesis_hash: alloy::primitives::B256::repeat_byte(0x01),
            registry: alloy::primitives::Address::repeat_byte(0x02),
            registry_code_hash: alloy::primitives::B256::repeat_byte(0x03),
            anchor_rpcs: vec!["http://127.0.0.1:1".into(), "http://127.0.0.1:2".into()],
            anchor_operators: Vec::new(),
            snapshot_urls: Vec::new(),
            registration_meta_evidence: format!("/ipfs/{cid}/registration.json"),
            clearing_meta_evidence: format!("/ipfs/{cid}/clearing.json"),
            provider_rpc: None,
            gateways: Vec::new(),
            test_headerless_state_root: false,
        }
    }

    #[test]
    fn deployment_context_id_varies_with_every_trust_field() {
        // Round-6: the context id must change whenever ANY field defining
        // WHICH deployment/policy context varies — same-chain-id fork
        // genesis, same-address other codehash, edited policy pins, test
        // mode — and be stable across irrelevant fields (RPC endpoints).
        let base = base_profile();
        let base_id = base.deployment_context_id();
        assert_eq!(base_id, base_profile().deployment_context_id(), "stable");

        let mut p = base_profile();
        p.genesis_hash = alloy::primitives::B256::repeat_byte(0x99);
        assert_ne!(p.deployment_context_id(), base_id, "genesis");
        let mut p = base_profile();
        p.registry_code_hash = alloy::primitives::B256::repeat_byte(0x99);
        assert_ne!(p.deployment_context_id(), base_id, "codehash");
        let mut p = base_profile();
        p.registration_meta_evidence.push('x');
        assert_ne!(p.deployment_context_id(), base_id, "registration pin");
        let mut p = base_profile();
        p.clearing_meta_evidence.push('x');
        assert_ne!(p.deployment_context_id(), base_id, "clearing pin");
        let mut p = base_profile();
        p.test_headerless_state_root = true;
        assert_ne!(p.deployment_context_id(), base_id, "test mode");
        let mut p = base_profile();
        p.chain_id = 101;
        assert_ne!(p.deployment_context_id(), base_id, "chain id");
        let mut p = base_profile();
        p.registry = alloy::primitives::Address::repeat_byte(0x99);
        assert_ne!(p.deployment_context_id(), base_id, "registry");
        // Irrelevant to WHICH deployment: transport endpoints.
        let mut p = base_profile();
        p.anchor_rpcs.push("http://127.0.0.1:3".into());
        p.gateways.push("http://gw".into());
        assert_eq!(p.deployment_context_id(), base_id, "endpoints irrelevant");
    }

    #[test]
    fn deployment_context_id_matches_the_v1_golden_vector() {
        // Round-7 regression pin: the v1 hash ENCODING must never drift
        // silently — persisted ids would stop matching and every catalog and
        // lockfile entry would fail closed. Golden value computed
        // independently (standalone binary, same domain tag + field order)
        // for the fixed base_profile() inputs.
        assert_eq!(
            base_profile().deployment_context_id(),
            "0x253f2981e5d5c11df649ad0fcf6418bf61cdc0b2d902f9100466eee3dd2350aa"
                .parse::<alloy::primitives::B256>()
                .unwrap()
        );
    }

    #[test]
    fn meta_evidence_refs_follow_deploy_registry_canonicality() {
        // The kubo fixture root: a real canonical 59-char CIDv1.
        let cid = "bafybeidgtfsc2ro3pfmyggmbz4ea7xg7g4gpehqur7klaadtreyjz6s3fu";
        assert_eq!(cid.len(), 59);
        let ok = |s: &str| validate_meta_evidence_ref("f", s).unwrap();
        let err = |s: &str| format!("{:#}", validate_meta_evidence_ref("f", s).unwrap_err());
        ok(&format!("/ipfs/{cid}"));
        ok(&format!("/ipfs/{cid}/registration.json"));
        ok(&format!("/ipfs/{cid}/a/b.json"));
        assert!(err("").contains("/ipfs/"));
        assert!(err("ipfs://x").contains("/ipfs/"));
        assert!(
            err("/ipfs/bench/registration.json").contains("/ipfs/"),
            "too short"
        );
        // CIDv0/uppercase/base58 forms are non-canonical here.
        assert!(err(&format!("/ipfs/Q{}", &cid[1..])).contains("base32"));
        assert!(err(&format!("/ipfs/{}", cid.to_uppercase())).contains("base32"));
        // Bad char inside the CID (base32 excludes '0', '1', '8', '9').
        let bad = format!("/ipfs/b{}", "0".repeat(58));
        assert!(err(&bad).contains("canonical"));
        // Valid ALPHABET but not a canonical decode (round-4): all-'7' sets
        // nonzero trailing bits in the final base32 group — data_encoding
        // refuses the decode.
        let noncanonical = format!("/ipfs/b{}", "7".repeat(58));
        assert!(
            err(&noncanonical).contains("strictly decode"),
            "{}",
            err(&noncanonical)
        );
        // Valid alphabet + canonical bits but the WRONG leading bytes (not
        // CIDv1/sha2-256/32): encode 36 zero bytes.
        let wrong_prefix = format!(
            "/ipfs/b{}",
            data_encoding::BASE32_NOPAD
                .encode(&[0u8; 36])
                .to_lowercase()
        );
        assert!(
            err(&wrong_prefix).contains("strictly decode"),
            "{}",
            err(&wrong_prefix)
        );
        // Suffix rules: only a '/'-separated non-empty path may follow.
        assert!(err(&format!("/ipfs/{cid}x")).contains("suffix"));
        assert!(err(&format!("/ipfs/{cid}/")).contains("not be empty"));
        // Interior/trailing empty path segments (round-5).
        assert!(err(&format!("/ipfs/{cid}/a//b.json")).contains("empty segments"));
        assert!(err(&format!("/ipfs/{cid}/a/")).contains("empty segments"));
        // No TODO placeholders.
        assert!(err(&format!("/ipfs/{cid}/TODO.json")).contains("TODO"));
    }

    #[test]
    fn origins_fold_loopback_case_and_paths() {
        assert_eq!(
            normalized_origin("http://LOCALHOST:8547/x").unwrap(),
            "http://127.0.0.1:8547"
        );
        assert_eq!(
            normalized_origin("http://127.0.0.1:8547").unwrap(),
            normalized_origin("http://localhost:8547/other?q=1").unwrap()
        );
        assert_eq!(
            normalized_origin("https://Rpc.GnosisChain.com").unwrap(),
            "https://rpc.gnosischain.com:443"
        );
        assert_ne!(
            normalized_origin("http://127.0.0.1:8547").unwrap(),
            normalized_origin("http://127.0.0.1:8548").unwrap()
        );
    }
}
