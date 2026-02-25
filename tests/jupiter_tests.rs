use shred_watcher::jupiter::{self, parse_fixed_tail};
use solana_sdk::{
    hash::Hash,
    message::{Message, MessageHeader, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};
use std::str::FromStr;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Builds a legacy VersionedTransaction with the given static account keys
/// and no instructions.
fn make_tx(account_keys: Vec<Pubkey>) -> VersionedTransaction {
    VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::Legacy(Message {
            header: MessageHeader {
                num_required_signatures: 0,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys,
            recent_blockhash: Hash::default(),
            instructions: vec![],
        }),
    }
}

// ─── try_decode tests ─────────────────────────────────────────────────────────

#[test]
fn no_accounts_returns_none() {
    assert!(jupiter::try_decode(&make_tx(vec![])).is_none());
}

#[test]
fn non_jupiter_accounts_returns_none() {
    let tx = make_tx(vec![Pubkey::new_unique(), Pubkey::new_unique()]);
    assert!(jupiter::try_decode(&tx).is_none());
}

#[test]
fn jupiter_v6_in_accounts_but_no_instructions_returns_none() {
    // Jupiter is present in the account list but there are no instructions
    // referencing it, so try_decode should return None without panicking.
    let jup = Pubkey::from_str("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4").unwrap();
    assert!(jupiter::try_decode(&make_tx(vec![Pubkey::new_unique(), jup])).is_none());
}

#[test]
fn jupiter_v4_in_accounts_but_no_instructions_returns_none() {
    let jup = Pubkey::from_str("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB").unwrap();
    assert!(jupiter::try_decode(&make_tx(vec![Pubkey::new_unique(), jup])).is_none());
}

// ─── parse_fixed_tail tests ───────────────────────────────────────────────────

#[test]
fn parse_fixed_tail_valid() {
    let mut args = vec![0u8; 5]; // dummy route_plan prefix
    args.extend_from_slice(&1_000_000_u64.to_le_bytes()); // in_amount
    args.extend_from_slice(&499_500_u64.to_le_bytes());   // quoted_out_amount
    args.extend_from_slice(&50_u16.to_le_bytes());        // slippage_bps (0.5%)
    args.push(3_u8);                                      // platform_fee_bps

    let (in_amt, out_amt, slip, fee) = parse_fixed_tail(&args).unwrap();
    assert_eq!(in_amt,  1_000_000);
    assert_eq!(out_amt,   499_500);
    assert_eq!(slip,           50);
    assert_eq!(fee,             3);
}

#[test]
fn parse_fixed_tail_too_short_returns_none() {
    // 18 bytes is one short of the required 19
    assert!(parse_fixed_tail(&[0u8; 18]).is_none());
}

#[test]
fn parse_fixed_tail_exact_minimum() {
    // Exactly 19 bytes of zeros — should succeed and return all-zero fields
    let (in_amt, out_amt, slip, fee) = parse_fixed_tail(&[0u8; 19]).unwrap();
    assert_eq!(in_amt, 0);
    assert_eq!(out_amt, 0);
    assert_eq!(slip, 0);
    assert_eq!(fee, 0);
}

#[test]
fn parse_fixed_tail_ignores_leading_bytes() {
    // The prefix can be any length — only the last 19 bytes matter
    let mut args = vec![0xAA_u8; 100]; // 100 bytes of noise
    let tail_start = args.len() - 19;
    args[tail_start..tail_start + 8].copy_from_slice(&42_u64.to_le_bytes());
    let (in_amt, ..) = parse_fixed_tail(&args).unwrap();
    assert_eq!(in_amt, 42);
}
