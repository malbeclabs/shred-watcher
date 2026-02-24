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
