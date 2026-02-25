use shred_watcher::shred::{self, ShredKind};

// ─── Byte offsets (from the Solana shred spec) ────────────────────────────────
const VARIANT_OFFSET: usize = 64;
const SLOT_OFFSET: usize    = 65;
const INDEX_OFFSET: usize   = 73;
const FLAGS_OFFSET: usize   = 85;
const SIZE_OFFSET: usize    = 86;
const PAYLOAD_OFFSET: usize = 88;

const LEGACY_DATA: u8 = 0xA5;
const LEGACY_CODE: u8 = 0x5A;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Builds a minimal valid legacy data shred byte buffer.
fn make_data_buf(slot: u64, index: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
    let size = PAYLOAD_OFFSET + payload.len();
    let mut buf = vec![0u8; size];
    buf[VARIANT_OFFSET] = LEGACY_DATA;
    buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&slot.to_le_bytes());
    buf[INDEX_OFFSET..INDEX_OFFSET + 4].copy_from_slice(&index.to_le_bytes());
    buf[FLAGS_OFFSET] = flags;
    buf[SIZE_OFFSET..SIZE_OFFSET + 2].copy_from_slice(&(size as u16).to_le_bytes());
    buf[PAYLOAD_OFFSET..].copy_from_slice(payload);
    buf
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn parse_legacy_data_fields() {
    let raw = make_data_buf(42, 7, 0x00, b"hello");
    let s = shred::parse(&raw).unwrap();
    assert_eq!(s.slot, 42);
    assert_eq!(s.index, 7);
    assert_eq!(s.kind, ShredKind::Data);
    assert_eq!(s.payload, b"hello");
    assert!(!s.data_complete);
    assert!(!s.last_in_slot);
}

#[test]
fn parse_legacy_code_fields() {
    let mut buf = vec![0u8; PAYLOAD_OFFSET];
    buf[VARIANT_OFFSET] = LEGACY_CODE;
    buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&100u64.to_le_bytes());
    buf[INDEX_OFFSET..INDEX_OFFSET + 4].copy_from_slice(&3u32.to_le_bytes());
    let s = shred::parse(&buf).unwrap();
    assert_eq!(s.slot, 100);
    assert_eq!(s.index, 3);
    assert_eq!(s.kind, ShredKind::Code);
    assert!(s.payload.is_empty());
}

#[test]
fn parse_too_short_is_err() {
    assert!(shred::parse(&vec![0u8; 10]).is_err());
}

#[test]
fn parse_unknown_variant_is_err() {
    let mut buf = vec![0u8; PAYLOAD_OFFSET];
    buf[VARIANT_OFFSET] = 0xFF;
    assert!(shred::parse(&buf).is_err());
}

#[test]
fn data_complete_flag_set() {
    let raw = make_data_buf(1, 0, 0b0100_0000, &[]);
    let s = shred::parse(&raw).unwrap();
    assert!(s.data_complete);
    assert!(!s.last_in_slot);
}

#[test]
fn last_in_slot_flag_set() {
    // 0b1100_0000 sets both last_in_slot (bit 7) and data_complete (bit 6)
    let raw = make_data_buf(1, 0, 0b1100_0000, &[]);
    let s = shred::parse(&raw).unwrap();
    assert!(s.data_complete);
    assert!(s.last_in_slot);
}

#[test]
fn slot_and_index_roundtrip() {
    let raw = make_data_buf(123_456_789, 65535, 0x00, &[]);
    let s = shred::parse(&raw).unwrap();
    assert_eq!(s.slot, 123_456_789);
    assert_eq!(s.index, 65535);
}

#[test]
fn parse_merkle_data_shred() {
    // 0x80: bits 7-6 == 0b10 → Merkle data, proof_size nibble = 0
    let mut raw = make_data_buf(99, 2, 0x40, b"merkle payload");
    raw[VARIANT_OFFSET] = 0x80;
    let s = shred::parse(&raw).unwrap();
    assert_eq!(s.kind, ShredKind::Data);
    assert_eq!(s.slot, 99);
    assert_eq!(s.index, 2);
}

#[test]
fn parse_merkle_code_shred() {
    // 0x60: bits 7-6 == 0b01 → Merkle code
    let mut buf = vec![0u8; PAYLOAD_OFFSET];
    buf[VARIANT_OFFSET] = 0x60;
    buf[SLOT_OFFSET..SLOT_OFFSET + 8].copy_from_slice(&7u64.to_le_bytes());
    let s = shred::parse(&buf).unwrap();
    assert_eq!(s.kind, ShredKind::Code);
    assert!(s.payload.is_empty());
}
