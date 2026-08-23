//! Staged, machine-readable result (brief rule 7: never a single `verified: true`).

use alloy::primitives::{Address, B256, U256};
use serde::Serialize;

use crate::consensus::AnchorReport;
use crate::highwater::Outcome;
use crate::proof::VerifiedAccount;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub mode: &'static str,
    pub chain_id: u64,
    pub anchor_source: String,
    pub checkpoint: B256,
    pub stages: Vec<Stage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_beacon: Option<FinalizedBeacon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<Execution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<Vec<Storage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_water: Option<Outcome>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedBeacon {
    pub slot: u64,
    pub block_root: B256,
    pub sync_participation: u64,
    pub updates_applied: usize,
    pub bootstrap_slot: u64,
    pub checkpoint_age_secs: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
    pub timestamp: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub address: Address,
    pub code_hash: B256,
    pub storage_root: B256,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub slot: B256,
    pub value: U256,
    pub meaning: String,
}

impl Report {
    pub fn new(chain_id: u64, anchor_source: String, checkpoint: B256) -> Self {
        Self {
            mode: "strict-proof-spike",
            chain_id,
            anchor_source,
            checkpoint,
            stages: Vec::new(),
            finalized_beacon: None,
            execution: None,
            account: None,
            storage: None,
            high_water: None,
        }
    }

    pub fn stage_ok(&mut self, name: &'static str, detail: impl Into<String>) {
        self.stages.push(Stage {
            name,
            ok: true,
            detail: detail.into(),
        });
    }

    pub fn stage_fail(&mut self, name: &'static str, detail: impl Into<String>) {
        self.stages.push(Stage {
            name,
            ok: false,
            detail: detail.into(),
        });
    }

    pub fn record_anchor(&mut self, a: &AnchorReport) {
        self.finalized_beacon = Some(FinalizedBeacon {
            slot: a.finalized_beacon_slot,
            block_root: a.finalized_beacon_root,
            sync_participation: a.sync_participation,
            updates_applied: a.updates_applied,
            bootstrap_slot: a.bootstrap_slot,
            checkpoint_age_secs: a.checkpoint_age_secs,
        });
        self.execution = Some(Execution {
            block_number: a.execution.block_number,
            block_hash: a.execution.block_hash,
            state_root: a.execution.state_root,
            timestamp: a.execution.timestamp,
        });
    }

    pub fn record_account(&mut self, address: Address, v: &VerifiedAccount) {
        self.account = Some(Account {
            address,
            code_hash: v.code_hash,
            storage_root: v.storage_root,
        });
        self.storage = Some(
            v.storage
                .iter()
                .map(|s| Storage {
                    slot: s.slot,
                    value: s.value,
                    meaning: s.meaning.clone(),
                })
                .collect(),
        );
    }
}
