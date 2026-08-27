//! Name-based install resolution policy (PR #1 review): name uniqueness must
//! hold at the FRESH anchor, never merely in the saved catalog — a same-name
//! entry Registered after the catalog was verified would otherwise be
//! invisible to the selection while only the selected item gets a fresh
//! proof. The caller establishes the candidate set is COMPLETE at the fresh
//! anchor first (proven itemCount equality with the verified catalog: names
//! are immutable — `itemID = keccak256(descriptor)` — so no unseen same-name
//! item can exist without growing the count), then proves every candidate's
//! CURRENT status; this pure function applies the uniqueness rule.

use alloy::primitives::B256;
use eyre::{bail, Result};

/// Pick the single Registered candidate among `(itemID, FRESH status)` pairs;
/// anything other than exactly one fails closed with a by-itemID hint.
pub fn pick_unique_registered(fresh: &[(B256, u8)]) -> Result<usize> {
    let registered: Vec<usize> = fresh
        .iter()
        .enumerate()
        .filter(|(_, (_, s))| *s == 1)
        .map(|(i, _)| i)
        .collect();
    match registered.len() {
        1 => Ok(registered[0]),
        0 => bail!(
            "none of the {} same-name candidate(s) is Registered at the fresh anchor — \
             nothing to install (statuses may have changed since `intend update`)",
            fresh.len()
        ),
        k => bail!(
            "{k} same-name candidates are Registered at the fresh anchor — the name is \
             ambiguous NOW even if the catalog was unambiguous; install by 0x-itemID"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::pick_unique_registered;
    use alloy::primitives::B256;

    #[test]
    fn uniqueness_is_decided_by_fresh_statuses_only() {
        let a = B256::repeat_byte(0xaa);
        let b = B256::repeat_byte(0xbb);
        // Exactly one Registered: picked, wherever it sits.
        assert_eq!(pick_unique_registered(&[(a, 2), (b, 1)]).unwrap(), 1);
        assert_eq!(pick_unique_registered(&[(a, 1)]).unwrap(), 0);
        // None Registered: refused.
        let err = format!(
            "{:#}",
            pick_unique_registered(&[(a, 0), (b, 3)]).unwrap_err()
        );
        assert!(err.contains("none of the 2"), "{err}");
        // Two Registered — the stale-catalog trap this policy exists for:
        // BOTH are Registered at the fresh anchor, so the name is ambiguous
        // regardless of what the catalog believed.
        let err = format!(
            "{:#}",
            pick_unique_registered(&[(a, 1), (b, 1)]).unwrap_err()
        );
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("0x-itemID"), "{err}");
    }
}
