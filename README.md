# shred-watcher

A high-performance Rust tool that listens to raw Solana shreds over UDP and detects **Jupiter DEX swaps** in real time — before transactions are confirmed on-chain.

## How it works

Solana validators broadcast block data as **shreds** via UDP (Turbine protocol). This tool:

1. Binds a UDP socket and receives shred packets from a validator.
2. Parses each packet as a Solana shred (legacy v1 and Merkle v2 formats).
3. Accumulates data shreds per slot and reassembles them into ledger entries.
4. Decodes each transaction looking for Jupiter v4/v6 swap instructions.
5. Logs detected swaps with signature, amounts, and slippage.

Because shreds arrive before a block is finalized, this gives you visibility into swaps **ahead of RPC confirmation**.

## Requirements

- Rust 1.70+
- A Solana validator (or turbine relay) forwarding shred UDP traffic to your machine
- Root or a raised `rmem_max` to set a large socket receive buffer:

```bash
# Allow up to 256 MB receive buffer (recommended)
sudo sysctl -w net.core.rmem_max=268435456
```

## Build

```bash
cargo build --release
```

## Usage

```
shred-watcher [OPTIONS]

Options:
  --bind <ADDR>       Listen address and port [default: 0.0.0.0:8001]
  --recv-buf <BYTES>  Kernel socket receive buffer size [default: 268435456 (256 MB)]
  --workers <N>       Number of parallel packet-processing workers [default: 4]
  -h, --help          Print help
```

### Examples

```bash
# Listen on all interfaces, port 8001 (default)
./target/release/shred-watcher

# Listen on a specific interface with 8 workers
./target/release/shred-watcher --bind 192.168.1.50:9000 --workers 8

# Verbose logging
RUST_LOG=debug ./target/release/shred-watcher
```

## Output

Each detected Jupiter swap is logged like this:

```
INFO  shred_watcher > 🪐 [slot=312847291] [JUP v6] sig=3xKpT7aQbcNv | JupiterSwap {
    instruction: "sharedAccountsRoute",
    in_amount: Some(5000000000),
    quoted_out_amount: Some(482317),
    slippage_bps: Some(50),
    platform_fee_bps: Some(0),
}
```

## Architecture

```
UDP socket (single reader)
        │
        ▼
  broadcast channel  ──► worker 0 ─┐
                     ──► worker 1 ─┤─► ShredAssembler ──► jupiter::try_decode ──► log
                     ──► worker N ─┘
```

| Module | Responsibility |
|---|---|
| `shred` | Parse raw UDP bytes into typed `Shred` structs (legacy + Merkle) |
| `assembler` | Buffer data shreds per slot; emit entries when slot is complete |
| `jupiter` | Match Anchor discriminators and decode swap arguments |

## Limitations

- **No erasure recovery**: if data shreds are lost in transit, the slot is dropped. Coding shreds are parsed but not used for FEC reconstruction.
- **Static accounts only**: Jupiter program detection checks the message's static account keys. Transactions using address table lookups to reference Jupiter may be missed.
- **Requires validator access**: you need a validator or relay that sends turbine traffic to your IP.

## License

MIT
