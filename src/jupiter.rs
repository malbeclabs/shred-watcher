/// Decoder for Jupiter Aggregator v6 instructions.
/// Program ID: JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4
use solana_sdk::{
    instruction::CompiledInstruction,
    pubkey::Pubkey,
    transaction::VersionedTransaction,
};
use std::str::FromStr;

// ─── Program IDs ──────────────────────────────────────────────────────────────

const JUP_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const JUP_V4: &str = "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB";

// ─── Anchor discriminators (sha256("global:<fn_name>")[0..8]) ─────────────────

const DISC_ROUTE: [u8; 8]                            = [0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a];
const DISC_SHARED_ACCOUNTS_ROUTE: [u8; 8]            = [0xc1, 0x20, 0x9b, 0x30, 0x75, 0x88, 0x08, 0x8f];
const DISC_EXACT_OUT_ROUTE: [u8; 8]                  = [0xd0, 0x33, 0xef, 0x97, 0x7b, 0x2b, 0xed, 0xd4];
const DISC_SHARED_ACCOUNTS_EXACT_OUT: [u8; 8]        = [0xb0, 0xd1, 0x69, 0xa8, 0x9a, 0x37, 0x8b, 0x8a];
const DISC_ROUTE_WITH_TOKEN_LEDGER: [u8; 8]          = [0x0e, 0xef, 0x71, 0x11, 0xdc, 0x55, 0x19, 0x06];
const DISC_SHARED_ACCOUNTS_ROUTE_WITH_LEDGER: [u8; 8]= [0x45, 0x08, 0x6a, 0xf2, 0xf3, 0xf6, 0x3d, 0x6e];

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug)]
#[allow(dead_code)]
pub struct JupiterSwap {
    pub instruction: &'static str,
    pub in_amount: Option<u64>,
    pub quoted_out_amount: Option<u64>,
    pub slippage_bps: Option<u16>,
    pub platform_fee_bps: Option<u8>,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Searches for a Jupiter instruction in the transaction and returns a
/// human-readable description if found.
pub fn try_decode(tx: &VersionedTransaction) -> Option<String> {
    let msg = &tx.message;
    let accounts = msg.static_account_keys();

    let jup_v6 = Pubkey::from_str(JUP_V6).unwrap();
    let jup_v4 = Pubkey::from_str(JUP_V4).unwrap();

    for ix in msg.instructions() {
        let prog_idx = ix.program_id_index as usize;
        let prog = accounts.get(prog_idx)?;

        if *prog != jup_v6 && *prog != jup_v4 {
            continue;
        }

        if let Some(decoded) = decode_instruction(ix, accounts) {
            let sig = tx.signatures.first()
                .map(|s| bs58::encode(s).into_string())
                .unwrap_or_else(|| "???".into());

            return Some(format!(
                "[{}] sig={} | {:?}",
                if *prog == jup_v6 { "JUP v6" } else { "JUP v4" },
                &sig[..12],
                decoded,
            ));
        }
    }
    None
}

// ─── Internals ────────────────────────────────────────────────────────────────

fn decode_instruction(
    ix: &CompiledInstruction,
    _accounts: &[Pubkey],
) -> Option<JupiterSwap> {
    let data = &ix.data;
    if data.len() < 8 {
        return None;
    }

    let disc: [u8; 8] = data[0..8].try_into().ok()?;
    let args = &data[8..];

    let name: &'static str = match disc {
        DISC_ROUTE                               => "route",
        DISC_SHARED_ACCOUNTS_ROUTE               => "sharedAccountsRoute",
        DISC_EXACT_OUT_ROUTE                     => "exactOutRoute",
        DISC_SHARED_ACCOUNTS_EXACT_OUT           => "sharedAccountsExactOutRoute",
        DISC_ROUTE_WITH_TOKEN_LEDGER             => "routeWithTokenLedger",
        DISC_SHARED_ACCOUNTS_ROUTE_WITH_LEDGER   => "sharedAccountsRouteWithTokenLedger",
        _                                        => return None,
    };

    // Jupiter v6 IDL layout for RouteArgs / SharedAccountsRouteArgs:
    // route_plan: Vec<RoutePlanStep>  — variable length, skip
    // in_amount: u64
    // quoted_out_amount: u64
    // slippage_bps: u16
    // platform_fee_bps: u8
    //
    // Strategy: read the fixed-size tail (last 19 bytes) since the
    // variable-length route_plan precedes the fixed fields.
    let (in_amount, quoted_out_amount, slippage_bps, platform_fee_bps) =
        parse_fixed_tail(args)?;

    Some(JupiterSwap {
        instruction: name,
        in_amount: Some(in_amount),
        quoted_out_amount: Some(quoted_out_amount),
        slippage_bps: Some(slippage_bps),
        platform_fee_bps: Some(platform_fee_bps),
    })
}

/// Reads the last 19 bytes of the instruction argument buffer:
/// [in_amount: u64][quoted_out_amount: u64][slippage_bps: u16][platform_fee_bps: u8]
/// These fixed fields always appear at the end, after the variable route_plan Vec.
fn parse_fixed_tail(args: &[u8]) -> Option<(u64, u64, u16, u8)> {
    const TAIL: usize = 8 + 8 + 2 + 1; // 19 bytes
    if args.len() < TAIL {
        return None;
    }
    let tail = &args[args.len() - TAIL..];
    let in_amount         = u64::from_le_bytes(tail[0..8].try_into().ok()?);
    let quoted_out_amount = u64::from_le_bytes(tail[8..16].try_into().ok()?);
    let slippage_bps      = u16::from_le_bytes(tail[16..18].try_into().ok()?);
    let platform_fee_bps  = tail[18];
    Some((in_amount, quoted_out_amount, slippage_bps, platform_fee_bps))
}
