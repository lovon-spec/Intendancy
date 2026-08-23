//! EIP-1186 account/storage proof verification against an authenticated state root.
//!
//! Mirrors the verification pattern of helios `core/src/execution/proof.rs` (same
//! `alloy_trie::proof::verify_proof` primitive; no bespoke MPT code), reimplemented
//! here so the spike depends only on `helios-consensus-core` + `alloy-trie`.
//!
//! Trust rule: `state_root` comes exclusively from the light-client-verified finalized
//! execution header; `expected_code_hash` and slot keys come exclusively from the local
//! fixture profile. The `EIP1186AccountProofResponse` itself is untrusted provider data.
//! Every trust decision here fails closed with a typed [`StageError`].

use alloy::consensus::TrieAccount;
use alloy::primitives::{keccak256, Bytes, B256, U256};
use alloy::rlp;
use alloy::rpc::types::EIP1186AccountProofResponse;
use alloy_trie::{proof::verify_proof, Nibbles};

use crate::errors::StageError;
use crate::profile::{Profile, SlotSpec};

#[derive(Debug, Clone)]
pub struct VerifiedStorage {
    pub slot: B256,
    pub value: U256,
    pub meaning: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedAccount {
    pub storage_root: B256,
    pub code_hash: B256,
    pub storage: Vec<VerifiedStorage>,
}

/// Verify the full response against the authenticated state root and the local profile.
/// Stages (each fails closed independently; see report):
///   1. account proof vs state root (address → RLP TrieAccount leaf)
///   2. code hash vs locally pinned runtime code hash
///   3. every profile-requested slot present, proven, and decodable
pub fn verify_response(
    proof: &EIP1186AccountProofResponse,
    state_root: B256,
    profile: &Profile,
) -> Result<VerifiedAccount, StageError> {
    if proof.address != profile.address {
        return Err(StageError::WrongAddress {
            got: proof.address,
            want: profile.address,
        });
    }

    // 1. Account proof.
    let account = TrieAccount {
        nonce: proof.nonce,
        balance: proof.balance,
        storage_root: proof.storage_hash,
        code_hash: proof.code_hash,
    };
    verify_mpt(state_root, proof.address, account, &proof.account_proof)
        .map_err(|detail| StageError::AccountProof { detail })?;

    // 2. Pinned runtime code hash: layout constants are meaningless for other bytecode.
    if proof.code_hash != profile.runtime_code_hash {
        return Err(StageError::CodeHashMismatch {
            proven: proof.code_hash,
            pinned: profile.runtime_code_hash,
        });
    }

    // 3. Storage slots: locally derived keys only; every requested slot must be proven
    //    AND decodable per its declared meaning (undecodable values are errors, never
    //    reported as successes).
    let mut out = Vec::with_capacity(profile.slots.len());
    for spec in &profile.slots {
        let want_key = spec.derive();
        let sp = proof
            .storage_proof
            .iter()
            .find(|p| p.key.as_b256() == want_key)
            .ok_or_else(|| StageError::MissingSlotProof {
                slot: want_key,
                meaning: spec.describe(),
            })?;
        verify_mpt(proof.storage_hash, want_key, sp.value, &sp.proof).map_err(|detail| {
            StageError::StorageProof {
                slot: want_key,
                meaning: spec.describe(),
                detail,
            }
        })?;
        out.push(VerifiedStorage {
            slot: want_key,
            value: sp.value,
            meaning: describe_value(spec, sp.value)?,
        });
    }

    Ok(VerifiedAccount {
        storage_root: proof.storage_hash,
        code_hash: proof.code_hash,
        storage: out,
    })
}

/// Decode a Classic GTCR `Status` word. The enum occupies its own storage slot, so the
/// ENTIRE word must be one of 0..=3; anything else is an error (review fix 3).
pub fn decode_status(value: U256) -> Result<(u8, &'static str), StageError> {
    if value > U256::from(3u64) {
        return Err(StageError::UndecodableStatus { value });
    }
    let status = value.byte(0);
    let name = match status {
        0 => "Absent",
        1 => "Registered",
        2 => "RegistrationRequested",
        3 => "ClearingRequested",
        _ => unreachable!("bounded above"),
    };
    Ok((status, name))
}

fn describe_value(spec: &SlotSpec, value: U256) -> Result<String, StageError> {
    Ok(match spec {
        SlotSpec::ItemListLength => format!("{} = itemCount {}", spec.describe(), value),
        SlotSpec::ItemListIndex(_) => format!("{} = itemID {:#066x}", spec.describe(), value),
        SlotSpec::ItemStatus(_) => {
            let (status, name) = decode_status(value)?;
            format!("{} = status {status} ({name})", spec.describe())
        }
    })
}

/// keccak(key)-pathed MPT proof check; zero/empty values verify as exclusion proofs.
fn verify_mpt<K: AsRef<[u8]>, V: rlp::Encodable>(
    root: B256,
    raw_key: K,
    raw_value: V,
    proof: &[Bytes],
) -> Result<(), String> {
    let key = Nibbles::unpack(keccak256(raw_key));
    let encoded = rlp::encode(raw_value);
    // RLP of integer zero / empty string is the single byte 0x80: an empty slot, which
    // must be proven by exclusion rather than inclusion.
    let expected = if encoded.as_slice() == [rlp::EMPTY_STRING_CODE] {
        None
    } else {
        Some(encoded)
    };
    verify_proof(root, key, expected, proof).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_zero_through_three_decode() {
        assert_eq!(decode_status(U256::from(0u64)).unwrap(), (0, "Absent"));
        assert_eq!(decode_status(U256::from(1u64)).unwrap(), (1, "Registered"));
        assert_eq!(
            decode_status(U256::from(2u64)).unwrap(),
            (2, "RegistrationRequested")
        );
        assert_eq!(
            decode_status(U256::from(3u64)).unwrap(),
            (3, "ClearingRequested")
        );
    }

    #[test]
    fn out_of_range_status_is_an_error_not_a_string() {
        // Review fix 3: values outside 0..=3 must FAIL, not succeed with a label.
        assert!(matches!(
            decode_status(U256::from(4u64)),
            Err(StageError::UndecodableStatus { .. })
        ));
        // Garbage in the upper bytes with a valid low byte must also fail: the enum
        // occupies the whole word.
        assert!(matches!(
            decode_status(U256::from(1u64) << 8),
            Err(StageError::UndecodableStatus { .. })
        ));
        assert!(decode_status(U256::MAX).is_err());
    }
}
