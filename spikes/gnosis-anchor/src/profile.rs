//! Local fixture profiles: the trusted, locally pinned side of verification.
//!
//! Everything here — target address, expected runtime codehash, storage-layout
//! constants, and requested slot keys — is LOCAL configuration. Nothing in this
//! module may ever be populated from provider output (RFC 0001 §2, brief §"Required
//! interface" rule 5).

use alloy::primitives::{address, b256, keccak256, Address, B256, U256};

/// Classic GeneralizedTCR storage layout (pinned bytecode; see results doc for the
/// derivation): `itemList` (bytes32[]) at slot 13, `items` mapping at slot 14.
/// `items[id]` base slot B holds the `data` bytes head; `status` is at B + 1, and the
/// WHOLE slot word must decode to 0..=3 (see `proof::decode_status`).
pub const ITEM_LIST_SLOT: u64 = 13;
pub const ITEMS_MAPPING_SLOT: u64 = 14;

#[derive(Debug, Clone, PartialEq)]
pub enum SlotSpec {
    /// `itemList.length` — raw slot 13.
    ItemListLength,
    /// `itemList[i]` — keccak256(uint256(13)) + i.
    ItemListIndex(u64),
    /// `items[itemID].status` — keccak256(abi.encode(itemID, uint256(14))) + 1.
    ItemStatus(B256),
}

impl SlotSpec {
    /// Derive the storage slot key locally. Provider-supplied slot keys are never used.
    pub fn derive(&self) -> B256 {
        match self {
            SlotSpec::ItemListLength => B256::from(U256::from(ITEM_LIST_SLOT)),
            SlotSpec::ItemListIndex(i) => {
                let base = keccak256(B256::from(U256::from(ITEM_LIST_SLOT)));
                B256::from(U256::from_be_bytes(base.0) + U256::from(*i))
            }
            SlotSpec::ItemStatus(item_id) => {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(item_id.as_slice());
                buf[32..].copy_from_slice(B256::from(U256::from(ITEMS_MAPPING_SLOT)).as_slice());
                let base = keccak256(buf);
                B256::from(U256::from_be_bytes(base.0) + U256::from(1u64))
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            SlotSpec::ItemListLength => "itemList.length (slot 13)".into(),
            SlotSpec::ItemListIndex(i) => format!("itemList[{i}] = keccak256(uint256(13)) + {i}"),
            SlotSpec::ItemStatus(id) => {
                format!("items[{id}].status = keccak256(abi.encode(id, uint256(14))) + 1")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: &'static str,
    pub chain_id: u64,
    pub address: Address,
    /// keccak256 of the deployed runtime code. The account proof's code hash MUST equal
    /// this pinned value before any storage-layout constant is applied.
    pub runtime_code_hash: B256,
    pub slots: Vec<SlotSpec>,
}

/// The known canary from the implementation brief: Classic GTCR factory instance 3 on
/// Gnosis (a live third-party registry with 27 items at the seed anchor). Seed values are
/// re-verified, not trusted; see the results doc for provenance checks.
pub fn canary() -> Profile {
    Profile {
        name: "gnosis-classic-gtcr-instance3",
        chain_id: crate::gnosis::CHAIN_ID,
        address: address!("54A92C21c6553a8085066311F2C8D9Db1B5e6610"),
        runtime_code_hash: b256!(
            "5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d"
        ),
        slots: vec![SlotSpec::ItemListLength, SlotSpec::ItemListIndex(0)],
    }
}

pub fn by_name(name: &str) -> Option<Profile> {
    match name {
        "gnosis-classic-gtcr-instance3" => Some(canary()),
        _ => None,
    }
}
