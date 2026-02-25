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
const LEGACY_DATA: u8 = 0b1010_0101; // 0xA5
const LEGACY_CODE: u8 = 0b0101_1010; // 0x5A
// Merkle variants are identified by the top two bits of the variant byte:
//   bits 7-6 == 0b10 (0x80-0xBF) → Merkle data
//   bits 7-6 == 0b01 (0x40-0x7F) → Merkle code
// The bottom 4 bits encode the proof_size; bits 5-4 encode chained/resigned flags.
const VARIANT_MASK: u8  = 0xC0;
const MERKLE_DATA_TAG: u8 = 0x80;
const MERKLE_CODE_TAG: u8 = 0x40;

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
        v if v & VARIANT_MASK == MERKLE_DATA_TAG => Ok(ShredKind::Data),
        v if v & VARIANT_MASK == MERKLE_CODE_TAG => Ok(ShredKind::Code),
        _ => bail!("unknown ShredVariant: 0x{v:02x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal valid legacy data shred byte buffer.
    fn make_data_buf(slot: u64, index: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
        let size = DATA_HEADER_SIZE + payload.len();
        let mut buf = vec![0u8; size];
        buf[SHRED_VARIANT_OFFSET] = LEGACY_DATA;
        buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&slot.to_le_bytes());
        buf[INDEX_OFFSET..INDEX_OFFSET + 4].copy_from_slice(&index.to_le_bytes());
        buf[DATA_FLAGS_OFFSET] = flags;
        buf[DATA_SIZE_OFFSET..DATA_SIZE_OFFSET + 2].copy_from_slice(&(size as u16).to_le_bytes());
        buf[DATA_HEADER_SIZE..].copy_from_slice(payload);
        buf
    }

    #[test]
    fn parse_legacy_data_fields() {
        let raw = make_data_buf(42, 7, 0x00, b"hello");
        let s = parse(&raw).unwrap();
        assert_eq!(s.slot, 42);
        assert_eq!(s.index, 7);
        assert_eq!(s.kind, ShredKind::Data);
        assert_eq!(s.payload, b"hello");
        assert!(!s.data_complete);
        assert!(!s.last_in_slot);
    }

    #[test]
    fn parse_legacy_code_fields() {
        let mut buf = vec![0u8; DATA_HEADER_SIZE];
        buf[SHRED_VARIANT_OFFSET] = LEGACY_CODE;
        buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&100u64.to_le_bytes());
        buf[INDEX_OFFSET..INDEX_OFFSET + 4].copy_from_slice(&3u32.to_le_bytes());
        let s = parse(&buf).unwrap();
        assert_eq!(s.slot, 100);
        assert_eq!(s.index, 3);
        assert_eq!(s.kind, ShredKind::Code);
        assert!(s.payload.is_empty());
    }

    #[test]
    fn parse_too_short_is_err() {
        assert!(parse(&vec![0u8; 10]).is_err());
    }

    #[test]
    fn parse_unknown_variant_is_err() {
        let mut buf = vec![0u8; DATA_HEADER_SIZE];
        buf[SHRED_VARIANT_OFFSET] = 0xFF;
        assert!(parse(&buf).is_err());
    }

    #[test]
    fn data_complete_flag_set() {
        let raw = make_data_buf(1, 0, 0b0100_0000, &[]);
        let s = parse(&raw).unwrap();
        assert!(s.data_complete);
        assert!(!s.last_in_slot);
    }

    #[test]
    fn last_in_slot_flag_set() {
        // 0b1100_0000 sets both last_in_slot (bit 7) and data_complete (bit 6)
        let raw = make_data_buf(1, 0, 0b1100_0000, &[]);
        let s = parse(&raw).unwrap();
        assert!(s.data_complete);
        assert!(s.last_in_slot);
    }

    #[test]
    fn slot_and_index_roundtrip() {
        let raw = make_data_buf(123_456_789, 65535, 0x00, &[]);
        let s = parse(&raw).unwrap();
        assert_eq!(s.slot, 123_456_789);
        assert_eq!(s.index, 65535);
    }

    #[test]
    fn parse_merkle_data_shred() {
        // 0x80 = Merkle data variant with proof_size nibble = 0
        let mut raw = make_data_buf(99, 2, 0x40, b"merkle payload");
        raw[SHRED_VARIANT_OFFSET] = 0x80;
        let s = parse(&raw).unwrap();
        assert_eq!(s.kind, ShredKind::Data);
        assert_eq!(s.slot, 99);
        assert_eq!(s.index, 2);
    }

    #[test]
    fn parse_merkle_code_shred() {
        // 0x60 = Merkle code variant
        let mut buf = vec![0u8; DATA_HEADER_SIZE];
        buf[SHRED_VARIANT_OFFSET] = 0x60;
        buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&7u64.to_le_bytes());
        let s = parse(&buf).unwrap();
        assert_eq!(s.kind, ShredKind::Code);
        assert!(s.payload.is_empty());
    }
}
