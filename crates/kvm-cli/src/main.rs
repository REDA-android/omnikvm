mod daemon;
#[allow(dead_code)]
mod router;
mod secure;
mod tools;
mod transfer;

use clap::{Parser, Subcommand};
use kvm_input_hal::linux::{detect_session_backend, UinputDevice};
use kvm_input_hal::mock::MockHal;
use kvm_input_hal::{InputEvent, SystemInputInjector};
use kvm_omni_core::crypto::{build_pairing_uri, generate_pairing_pin};
use kvm_omni_core::protocol::{flags, MouseMoveDeltaPayload, OmniPacket, PayloadType};
use kvm_omni_core::topology::{ScreenRect, ScreenTopology};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "omnikvm", version, about = "OmniKVM Hybrid Software KVM CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print detected OS, HAL backend, and protocol status
    Status,
    /// Start the live QUIC HID receiver daemon
    Serve {
        #[arg(long, default_value = "0.0.0.0:4433")]
        bind: SocketAddr,
        #[arg(long)]
        mock: bool,
        /// Pairing PIN: enables SPAKE2+/TOFU auth + sealed (MAC/anti-replay) datagrams
        #[arg(long, env = "OMNIKVM_PIN")]
        pin: Option<String>,
        #[arg(long, default_value = "omnikvm.json")]
        config: std::path::PathBuf,
    },
    /// Connect to a remote OmniKVM peer and send synthetic HID events
    Connect {
        #[arg(long)]
        peer: SocketAddr,
        #[arg(long, default_value_t = 10)]
        dx: i32,
        #[arg(long, default_value_t = 0)]
        dy: i32,
        /// Pairing PIN shown by the server (`omnikvm pair`)
        #[arg(long, env = "OMNIKVM_PIN")]
        pin: Option<String>,
        #[arg(long, default_value = "omnikvm.json")]
        config: std::path::PathBuf,
    },
    /// Generate a SPAKE2+ pairing PIN and URI
    Pair {
        #[arg(long, default_value = "omnikvm-host")]
        peer_id: String,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 4433)]
        port: u16,
    },
    /// Benchmark OmniKVM Wire Protocol v1 serialize/deserialize latency
    Bench {
        #[arg(long, default_value_t = 1_000_000)]
        count: usize,
    },
    /// Evaluate a cursor move on a sample dual-screen topology
    TopologyCheck {
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
    },
    /// Send a file over QUIC using 64 KB Blake3-XOF encrypted chunks
    SendFile {
        #[arg(long)]
        peer: SocketAddr,
        #[arg(long)]
        file: std::path::PathBuf,
    },
    /// Switch a monitor input over DDC/CI (VCP 0x60) via /dev/i2c-N
    DdcSwitch {
        /// dp1 | dp2 | hdmi1 | hdmi2 | usbc
        #[arg(long)]
        input: String,
        #[arg(long)]
        bus: Option<std::path::PathBuf>,
        /// Only print the MCCS frame
        #[arg(long)]
        dry_run: bool,
    },
    /// List I2C buses and DDC/CI input codes
    DdcList,
    /// Classify stdin clipboard content; --osc52 emits an SSH/tmux-friendly copy sequence
    Clipboard {
        #[arg(long)]
        osc52: bool,
    },
    /// Capture a local evdev device and forward it to an authenticated peer
    Forward {
        #[arg(long)]
        device: std::path::PathBuf,
        #[arg(long)]
        peer: SocketAddr,
        #[arg(long, env = "OMNIKVM_PIN")]
        pin: String,
        #[arg(long, default_value = "omnikvm.json")]
        config: std::path::PathBuf,
        /// Exclusive EVIOCGRAB (local apps stop receiving input)
        #[arg(long)]
        grab: bool,
    },
    /// Receive an encrypted QUIC file transfer and verify its Blake3 digest
    RecvFile {
        #[arg(long, default_value = "0.0.0.0:4435")]
        bind: SocketAddr,
        #[arg(long)]
        out: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Status => {
            let backend = detect_session_backend();
            println!("OmniKVM v{}", env!("CARGO_PKG_VERSION"));
            println!("OS: {}", std::env::consts::OS);
            println!("Linux HAL Backend: {backend:?}");
        }
        Commands::Serve {
            bind,
            mock,
            pin,
            config,
        } => {
            let injector: Arc<dyn SystemInputInjector> = if mock {
                Arc::new(MockHal::new())
            } else {
                match UinputDevice::open_default() {
                    Ok(dev) => Arc::new(dev),
                    Err(e) => {
                        eprintln!("WARN: /dev/uinput unavailable ({e}), falling back to MockHal");
                        Arc::new(MockHal::new())
                    }
                }
            };
            if let Some(pin) = pin {
                let opts = secure::SecureOpts {
                    pin,
                    config_path: config,
                };
                let (_addr, handle) = secure::run_secure_server(bind, injector, opts, None).await?;
                let stats = handle.await??;
                println!("Secure session stats: {stats:?}");
            } else {
                eprintln!("WARN: no --pin given, running UNAUTHENTICATED (LAN test mode)");
                let (_addr, handle) = daemon::run_server(bind, injector, None).await?;
                handle.await??;
            }
        }
        Commands::Connect {
            peer,
            dx,
            dy,
            pin,
            config,
        } => {
            let events = vec![InputEvent::MouseMove { dx, dy }];
            let sent = match pin {
                Some(pin) => {
                    let opts = secure::SecureOpts {
                        pin,
                        config_path: config,
                    };
                    secure::send_events_secure(peer, &events, &opts).await?
                }
                None => daemon::send_events_to_peer(peer, &events).await?,
            };
            println!("Sent {sent} HID event(s) over QUIC to {peer}");
        }
        Commands::Pair {
            peer_id,
            host,
            port,
        } => {
            let pin = generate_pairing_pin();
            let uri = build_pairing_uri(&peer_id, &host, port, &pin);
            println!("PIN: {pin}");
            println!("URI: {uri}");
        }
        Commands::Bench { count } => {
            let pkt = OmniPacket::from_typed(
                flags::URGENT_HID,
                1,
                1,
                0,
                PayloadType::MouseMoveDelta,
                &MouseMoveDeltaPayload { dx: 3, dy: -2 },
            )?;
            let mut buf = Vec::with_capacity(64);
            let start = Instant::now();
            for _ in 0..count {
                pkt.encode_into(&mut buf);
                let parsed = OmniPacket::deserialize(&buf)?;
                std::hint::black_box(parsed);
            }
            let elapsed = start.elapsed();
            let ns_per_op = elapsed.as_nanos() as f64 / count as f64;
            println!("Completed {count} roundtrips in {elapsed:?} ({ns_per_op:.1} ns/op)");
        }
        Commands::TopologyCheck { x, y } => {
            let mut topo = ScreenTopology::default();
            topo.add_screen(ScreenRect {
                id: "primary-4k".into(),
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
                dpi_scale: 2.0,
            });
            topo.add_screen(ScreenRect {
                id: "secondary-fhd".into(),
                x: 3840,
                y: 540,
                width: 1920,
                height: 1080,
                dpi_scale: 1.0,
            });
            match topo.evaluate_move("primary-4k", x, y) {
                Some(sw) => println!(
                    "Switch -> {} at ({}, {})",
                    sw.target_screen, sw.entry_x, sw.entry_y
                ),
                None => println!("No screen switch at ({x}, {y})"),
            }
        }
        Commands::SendFile { peer, file } => {
            let digest = transfer::send_file_quic(peer, &file, [0x42u8; 32]).await?;
            println!("File sent & Blake3 verified: {:02x?}", &digest[..8]);
        }
        Commands::DdcSwitch {
            input,
            bus,
            dry_run,
        } => tools::ddc_switch(&input, bus.as_deref(), dry_run)?,
        Commands::DdcList => tools::ddc_list(),
        Commands::Clipboard { osc52 } => tools::clipboard_from_stdin(osc52)?,
        Commands::Forward {
            device,
            peer,
            pin,
            config,
            grab,
        } => {
            let opts = secure::SecureOpts {
                pin,
                config_path: config,
            };
            tools::forward_device(device, peer, opts, grab).await?;
        }
        Commands::RecvFile { bind, out } => {
            let (addr, handle) = transfer::recv_file_quic(bind, &out, [0x42u8; 32]).await?;
            println!("Listening for encrypted QUIC file transfer on {addr}...");
            let digest = handle.await??;
            println!("Received & Blake3 verified: {:02x?}", &digest[..8]);
        }
    }
    Ok(())
}
