use anyhow::Result;
use dashmap::DashMap;
/// Accumulates Data shreds by slot and reconstructs Entries once the slot is
/// marked as data_complete or last_in_slot.
use std::collections::BTreeMap;
use tracing::{debug, warn};

use crate::shred::{Shred, ShredKind};
use solana_sdk::transaction::VersionedTransaction;

// ─── Public types ─────────────────────────────────────────────────────────────

pub struct Entry {
    pub slot: u64,
    pub transactions: Vec<VersionedTransaction>,
}

// ─── Assembler ────────────────────────────────────────────────────────────────

pub struct ShredAssembler {
    // slot → sorted map of shred_index → payload bytes
    buffers: DashMap<u64, BTreeMap<u32, Vec<u8>>>,
    // slot → whether a data_complete signal has been received
    complete: DashMap<u64, bool>,
}

impl ShredAssembler {
    pub fn new() -> Self {
        Self {
            buffers: DashMap::new(),
            complete: DashMap::new(),
        }
    }

    /// Insert a shred. Returns assembled entries if the slot is ready.
    pub fn push(&mut self, shred: Shred) -> Option<Vec<Entry>> {
        if shred.kind != ShredKind::Data || shred.payload.is_empty() {
            return None;
        }

        let slot = shred.slot;
        let done = shred.data_complete || shred.last_in_slot;

        self.buffers
            .entry(slot)
            .or_default()
            .insert(shred.index, shred.payload);

        if done {
            self.complete.insert(slot, true);
        }

        if self.complete.contains_key(&slot) {
            self.try_assemble(slot)
        } else {
            None
        }
    }

    fn try_assemble(&mut self, slot: u64) -> Option<Vec<Entry>> {
        let (_, frags) = self.buffers.remove(&slot)?;
        self.complete.remove(&slot);

        // Concatenate payloads in index order
        let blob: Vec<u8> = frags.into_values().flatten().collect();

        // Entries are bincode-encoded inside the blob
        match deserialize_entries(&blob) {
            Ok(txs) => {
                debug!("Slot {slot}: {} transactions deserialized", txs.len());
                Some(vec![Entry {
                    slot,
                    transactions: txs,
                }])
            }
            Err(e) => {
                warn!("Slot {slot}: failed to deserialize entries: {e}");
                None
            }
        }
    }
}

/// Deserializes the blob for a slot into a flat list of VersionedTransactions.
/// Solana encodes slot data as a bincode Vec<Entry>, where each Entry contains:
/// num_hashes(u64) + hash(32 bytes) + Vec<VersionedTransaction>.
fn deserialize_entries(data: &[u8]) -> Result<Vec<VersionedTransaction>> {
    let entries: Vec<solana_entry::entry::Entry> = bincode::deserialize(data)?;
    let txs = entries.into_iter().flat_map(|e| e.transactions).collect();
    Ok(txs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shred::{Shred, ShredKind};

    fn data_shred(slot: u64, index: u32, data_complete: bool, payload: Vec<u8>) -> Shred {
        Shred {
            slot,
            index,
            version: 0,
            fec_set_index: 0,
            kind: ShredKind::Data,
            payload,
            last_in_slot: data_complete,
            data_complete,
            parent_offset: 0,
        }
    }

    fn code_shred(slot: u64, index: u32) -> Shred {
        Shred {
            slot,
            index,
            version: 0,
            fec_set_index: 0,
            kind: ShredKind::Code,
            payload: vec![],
            last_in_slot: false,
            data_complete: false,
            parent_offset: 0,
        }
    }

    #[test]
    fn code_shred_is_ignored() {
        let mut asm = ShredAssembler::new();
        assert!(asm.push(code_shred(1, 0)).is_none());
    }

    #[test]
    fn empty_payload_is_ignored() {
        let mut asm = ShredAssembler::new();
        assert!(asm.push(data_shred(1, 0, true, vec![])).is_none());
    }

    #[test]
    fn incomplete_slot_returns_none() {
        let mut asm = ShredAssembler::new();
        assert!(asm.push(data_shred(10, 0, false, vec![1, 2, 3])).is_none());
        assert!(asm.push(data_shred(10, 1, false, vec![4, 5, 6])).is_none());
    }

    #[test]
    fn complete_flag_triggers_assembly_attempt() {
        let mut asm = ShredAssembler::new();
        // Invalid bincode payload — deserialization fails gracefully, no panic.
        let result = asm.push(data_shred(5, 0, true, vec![0xFF; 32]));
        assert!(result.is_none());
    }

    #[test]
    fn out_of_order_arrival() {
        let mut asm = ShredAssembler::new();
        // Shred 1 (marked complete) arrives first — assembly is attempted immediately.
        assert!(asm.push(data_shred(20, 1, true, vec![0xBB])).is_none());
        // Shred 0 arrives after — slot was already flushed, so None again.
        assert!(asm.push(data_shred(20, 0, false, vec![0xAA])).is_none());
    }

    #[test]
    fn independent_slots_dont_interfere() {
        let mut asm = ShredAssembler::new();
        asm.push(data_shred(1, 0, false, vec![0x01]));
        asm.push(data_shred(2, 0, true,  vec![0x02])); // slot 2 triggers assembly
        // slot 2 flushed; slot 1 still buffered — completing it triggers its own assembly
        assert!(asm.push(data_shred(1, 1, true, vec![0x03])).is_none());
    }

    #[test]
    fn multiple_code_shreds_all_ignored() {
        let mut asm = ShredAssembler::new();
        for i in 0..5 {
            assert!(asm.push(code_shred(99, i)).is_none());
        }
    }
}
