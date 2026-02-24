/// Manual parser for Solana shreds (legacy v1 and Merkle v2 format).
/// Spec: https://github.com/solana-labs/solana/blob/master/ledger/src/shred.rs
use anyhow::{bail, Result};

// ─── Format constants ─────────────────────────────────────────────────────────
const SIGNATURE_LEN: usize = 64;
const SHRED_VARIANT_OFFSET: usize = SIGNATURE_LEN; // byte 64
const SLOT_OFFSET: usize = 65;                      // bytes 65-72
const INDEX_OFFSET: usize = 73;                     // bytes 73-76
const VERSION_OFFSET: usize = 77;                   // bytes 77-78
const FEC_SET_INDEX_OFFSET: usize = 79;             // bytes 79-82

// Data shred (legacy): header ends at byte 87, payload starts at 88
const DATA_PARENT_OFFSET: usize = 83;
const DATA_FLAGS_OFFSET: usize = 85;
const DATA_SIZE_OFFSET: usize = 86;
const DATA_HEADER_SIZE: usize = 88;

// ShredVariant byte values
const LEGACY_DATA: u8 = 0b1010_0101;
const LEGACY_CODE: u8 = 0b0101_1010;
const MERKLE_DATA_BASE: u8 = 0x40; // 0x40..=0x4F
const MERKLE_CODE_BASE: u8 = 0x60; // 0x60..=0x6F

#[derive(Debug, Clone, PartialEq)]
pub enum ShredKind {
    Data,
    Code,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Shred {
    pub slot: u64,
    pub index: u32,
    pub version: u16,
    pub fec_set_index: u32,
    pub kind: ShredKind,
    /// Raw data payload (only populated for ShredKind::Data)
    pub payload: Vec<u8>,
    /// True if this is the last shred in the slot
    pub last_in_slot: bool,
    /// True if this is the last data shred in the FEC set
    pub data_complete: bool,
    pub parent_offset: u16,
}

pub fn parse(raw: &[u8]) -> Result<Shred> {
    if raw.len() < DATA_HEADER_SIZE {
        bail!("shred too short: {} bytes", raw.len());
    }

    let variant = raw[SHRED_VARIANT_OFFSET];
    let kind = classify_variant(variant)?;

    let slot = u64::from_le_bytes(raw[SLOT_OFFSET..SLOT_OFFSET + 8].try_into()?);
    let index = u32::from_le_bytes(raw[INDEX_OFFSET..INDEX_OFFSET + 4].try_into()?);
    let version = u16::from_le_bytes(raw[VERSION_OFFSET..VERSION_OFFSET + 2].try_into()?);
    let fec_set_index =
        u32::from_le_bytes(raw[FEC_SET_INDEX_OFFSET..FEC_SET_INDEX_OFFSET + 4].try_into()?);

    let (payload, last_in_slot, data_complete, parent_offset) = if kind == ShredKind::Data {
        let parent_offset =
            u16::from_le_bytes(raw[DATA_PARENT_OFFSET..DATA_PARENT_OFFSET + 2].try_into()?);
        let flags = raw[DATA_FLAGS_OFFSET];
        let size =
            u16::from_le_bytes(raw[DATA_SIZE_OFFSET..DATA_SIZE_OFFSET + 2].try_into()?) as usize;

        let last_in_slot = flags & 0x80 != 0;
        let data_complete = flags & 0x40 != 0;

        let end = DATA_HEADER_SIZE + size.min(raw.len() - DATA_HEADER_SIZE);
        let payload = raw[DATA_HEADER_SIZE..end].to_vec();

        (payload, last_in_slot, data_complete, parent_offset)
    } else {
        (vec![], false, false, 0u16)
    };

    Ok(Shred {
        slot,
        index,
        version,
        fec_set_index,
        kind,
        payload,
        last_in_slot,
        data_complete,
        parent_offset,
    })
}

fn classify_variant(v: u8) -> Result<ShredKind> {
    match v {
        LEGACY_DATA => Ok(ShredKind::Data),
        LEGACY_CODE => Ok(ShredKind::Code),
        v if (MERKLE_DATA_BASE..MERKLE_DATA_BASE + 0x10).contains(&v) => Ok(ShredKind::Data),
        v if (MERKLE_CODE_BASE..MERKLE_CODE_BASE + 0x10).contains(&v) => Ok(ShredKind::Code),
        _ => bail!("unknown ShredVariant: 0x{v:02x}"),
    }
}
