use shred_watcher::assembler::ShredAssembler;
use shred_watcher::shred::{Shred, ShredKind};

// ─── Helpers ──────────────────────────────────────────────────────────────────

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

// ─── Tests ────────────────────────────────────────────────────────────────────

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
