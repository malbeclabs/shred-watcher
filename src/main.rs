mod shred;
mod assembler;
mod jupiter;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn, debug, error};

#[derive(Parser)]
#[command(name = "shred-watcher", about = "Listens for UDP shreds from Solana validators")]
struct Cli {
    /// Bind address and port. E.g. "0.0.0.0:8001" or "192.168.1.50:9000"
    #[arg(long, default_value = "0.0.0.0:8001")]
    bind: String,

    /// Socket receive buffer size in bytes (default: 256 MB)
    #[arg(long, default_value_t = 256 * 1024 * 1024)]
    recv_buf: usize,

    /// Number of parallel packet-processing workers
    #[arg(long, default_value_t = 4)]
    workers: usize,

    /// Network interface to bind to (e.g. "eth0", "enp3s0").
    /// Uses SO_BINDTODEVICE — requires root or CAP_NET_RAW.
    #[arg(long)]
    iface: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "shred_watcher=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Bind the UDP socket
    let socket = UdpSocket::bind(&cli.bind).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            if let Ok(addr) = cli.bind.parse::<std::net::SocketAddr>() {
                if addr.port() < 1024 {
                    return anyhow::anyhow!(
                        "Cannot bind to {} — port {} is privileged (< 1024).\n\
                         Run with sudo, or lower the limit:\n\
                         \tsudo sysctl -w net.ipv4.ip_unprivileged_port_start={}",
                        cli.bind, addr.port(), addr.port()
                    );
                }
            }
            anyhow::anyhow!("Permission denied binding to {} — try running with sudo", cli.bind)
        } else {
            anyhow::anyhow!("Failed to bind to {}: {}", cli.bind, e)
        }
    })?;

    // Increase the kernel receive buffer to avoid dropping packets under bursts
    let raw_fd = {
        use std::os::unix::io::AsRawFd;
        socket.as_raw_fd()
    };
    set_socket_recv_buf(raw_fd, cli.recv_buf)?;

    if let Some(ref iface) = cli.iface {
        bind_to_iface(raw_fd, iface)?;
    }

    info!("✅ Listening for shreds on {}", cli.bind);
    if let Some(ref iface) = cli.iface {
        info!("   Interface: {iface}");
    }
    info!("   Socket buffer: {} MB", cli.recv_buf / 1024 / 1024);
    info!("   Workers: {}", cli.workers);

    // Share the socket across workers with Arc
    let socket = Arc::new(socket);
    let assembler = Arc::new(tokio::sync::Mutex::new(assembler::ShredAssembler::new()));

    // Internal channel to distribute packets to workers
    let (tx, _rx) = tokio::sync::broadcast::channel::<(Vec<u8>, std::net::SocketAddr)>(8192);

    // Spawn workers
    for worker_id in 0..cli.workers {
        let mut rx = tx.subscribe();
        let asm = Arc::clone(&assembler);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok((pkt, peer)) => process_packet(worker_id, &pkt, peer, &asm).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Worker {worker_id}: dropped {n} packets (lagged)");
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Main receive loop — a single thread reads from the socket
    let mut buf = vec![0u8; 1280]; // Maximum MTU for a Solana shred
    let mut total_pkts: u64 = 0;

    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, peer)) => {
                total_pkts += 1;
                if total_pkts % 10_000 == 0 {
                    info!("📦 Packets received: {total_pkts}");
                }
                // Forward a copy to the worker channel
                let _ = tx.send((buf[..len].to_vec(), peer));
            }
            Err(e) => {
                error!("Error receiving packet: {e}");
            }
        }
    }
}

async fn process_packet(
    worker_id: usize,
    raw: &[u8],
    peer: std::net::SocketAddr,
    assembler: &tokio::sync::Mutex<assembler::ShredAssembler>,
) {
    match shred::parse(raw) {
        Ok(shred) => {
            debug!(
                "[W{worker_id}] Shred from {peer} → slot={} idx={} kind={:?} payload={}B",
                shred.slot, shred.index, shred.kind, shred.payload.len()
            );

            let mut asm = assembler.lock().await;
            if let Some(entries) = asm.push(shred) {
                drop(asm); // release the lock before processing entries
                for entry in entries {
                    for tx in entry.transactions {
                        if let Some(decoded) = jupiter::try_decode(&tx) {
                            info!("🪐 [slot={}] {decoded}", entry.slot);
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("[W{worker_id}] Invalid packet from {peer} ({} bytes): {e}", raw.len());
        }
    }
}

/// Binds the socket to a specific network interface via SO_BINDTODEVICE.
/// Requires root or CAP_NET_RAW.
fn bind_to_iface(fd: std::os::unix::io::RawFd, iface: &str) -> Result<()> {
    use std::ffi::CString;
    let name = CString::new(iface)?;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr() as *const libc::c_void,
            iface.len() as libc::socklen_t,
        )
    };
    if ret != 0 {
        anyhow::bail!(
            "Failed to bind to interface '{}' (requires root or CAP_NET_RAW): {}",
            iface,
            std::io::Error::last_os_error()
        );
    }
    info!("   Bound to interface '{iface}' via SO_BINDTODEVICE");
    Ok(())
}

/// Sets SO_RCVBUF on the socket to handle shred bursts without packet loss.
fn set_socket_recv_buf(fd: std::os::unix::io::RawFd, size: usize) -> Result<()> {
    use std::mem::size_of;
    let size = size as libc::c_int;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &size as *const _ as *const libc::c_void,
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        warn!("Failed to set SO_RCVBUF (requires root or a raised /proc/sys/net/core/rmem_max)");
    }
    Ok(())
}


