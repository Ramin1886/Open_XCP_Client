// SPDX-License-Identifier: GPL-2.0-only
//! Asynchronous XCP-on-Ethernet (UDP and TCP) transport layer that demultiplexes
//! incoming packets into CTO command responses and DTO DAQ data streams so
//! calibration and recording operate concurrently without blocking.

use crate::config_store::TransportProtocol;
use crate::xcp::link_stats::LinkStats;
use crate::xcp::packet::{encode_xcp_ethernet_frame, unpack_xcp_ethernet_frames, XcpPacketKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{mpsc, Mutex as AsyncMutex};

/// Demultiplexed DTO DAQ packet delivered to the recording pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingDaqDto {
    pub odt_num: u8,
    pub daq_list_num: u8,
    pub payload: Vec<u8>,
    /// The 16-bit XCP-on-Ethernet `CTR` of the frame this DTO arrived in, kept for diagnostics.
    pub counter: u16,
    /// Value of [`LinkStats::generation`] when this DTO was received. A change between
    /// consecutive DTOs means the link lost, reordered or duplicated frames in between, so any
    /// half-assembled DAQ cycle must be abandoned.
    pub link_generation: u64,
}

enum TransportWriteHalf {
    Udp {
        socket: Arc<UdpSocket>,
        target_addr: SocketAddr,
    },
    Tcp {
        writer: tokio::net::tcp::OwnedWriteHalf,
    },
}

/// Asynchronous XCP transport supporting both UDP and TCP.
pub struct XcpTransport {
    writer: AsyncMutex<TransportWriteHalf>,
    cto_rx: AsyncMutex<mpsc::Receiver<XcpPacketKind>>,
    tx_counter: AtomicU16,
    closed: Arc<AtomicBool>,
    /// Link-integrity counters observed over **every** received frame.
    ///
    /// The `CTR` is shared by CTO responses and DAQ DTOs on one connection, so it can only be
    /// tracked here, where all frames pass. Observing it on DTOs alone would score every command
    /// response as a gap.
    link_stats: Arc<StdMutex<LinkStats>>,
}

impl XcpTransport {
    /// Connects to the XCP target at `addr` using either UDP or TCP and returns
    /// `(XcpTransport, mpsc::Receiver<IncomingDaqDto>)`.
    pub async fn connect(
        addr: SocketAddr,
        protocol: TransportProtocol,
    ) -> std::io::Result<(Arc<Self>, mpsc::Receiver<IncomingDaqDto>)> {
        let (cto_tx, cto_rx) = mpsc::channel::<XcpPacketKind>(256);
        let (daq_tx, daq_rx) = mpsc::channel::<IncomingDaqDto>(8192);
        let closed = Arc::new(AtomicBool::new(false));
        let link_stats = Arc::new(StdMutex::new(LinkStats::new()));

        match protocol {
            TransportProtocol::Udp => {
                let bind_addr: SocketAddr = if addr.is_ipv4() {
                    "0.0.0.0:0".parse().unwrap()
                } else {
                    "[::]:0".parse().unwrap()
                };
                let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
                socket.connect(addr).await?;
                let rx_sock = Arc::clone(&socket);
                let closed_clone = Arc::clone(&closed);
                let stats_clone = Arc::clone(&link_stats);

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    while !closed_clone.load(Ordering::Relaxed) {
                        match rx_sock.recv(&mut buf).await {
                            Ok(n) if n > 0 => {
                                let (frames, _) = unpack_xcp_ethernet_frames(&buf[..n]);
                                for frame in frames {
                                    dispatch_frame(&frame, &cto_tx, &daq_tx, &stats_clone).await;
                                }
                            }
                            Ok(_) => break,
                            Err(_) => break,
                        }
                    }
                });

                let transport = Arc::new(Self {
                    writer: AsyncMutex::new(TransportWriteHalf::Udp {
                        socket,
                        target_addr: addr,
                    }),
                    cto_rx: AsyncMutex::new(cto_rx),
                    tx_counter: AtomicU16::new(0),
                    closed,
                    link_stats,
                });
                Ok((transport, daq_rx))
            }
            TransportProtocol::Tcp => {
                let stream = TcpStream::connect(addr).await?;
                let _ = stream.set_nodelay(true);
                let (mut reader, writer) = stream.into_split();
                let closed_clone = Arc::clone(&closed);
                let stats_clone = Arc::clone(&link_stats);

                tokio::spawn(async move {
                    let mut stream_buf = Vec::with_capacity(65536);
                    let mut temp = [0u8; 8192];
                    while !closed_clone.load(Ordering::Relaxed) {
                        match reader.read(&mut temp).await {
                            Ok(0) => break,
                            Ok(n) => {
                                stream_buf.extend_from_slice(&temp[..n]);
                                let (frames, consumed) = unpack_xcp_ethernet_frames(&stream_buf);
                                if consumed > 0 {
                                    stream_buf.drain(0..consumed);
                                }
                                for frame in frames {
                                    dispatch_frame(&frame, &cto_tx, &daq_tx, &stats_clone).await;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                let transport = Arc::new(Self {
                    writer: AsyncMutex::new(TransportWriteHalf::Tcp { writer }),
                    cto_rx: AsyncMutex::new(cto_rx),
                    tx_counter: AtomicU16::new(0),
                    closed,
                    link_stats,
                });
                Ok((transport, daq_rx))
            }
        }
    }

    /// Returns a snapshot of the link-integrity counters observed over every received frame.
    pub fn link_stats(&self) -> LinkStats {
        self.link_stats
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Sends a CTO command payload and waits up to `timeout` for the corresponding response.
    pub async fn send_command(
        &self,
        cto_payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, crate::xcp::client::XcpError> {
        let ctr = self.tx_counter.fetch_add(1, Ordering::Relaxed);
        let raw_frame = encode_xcp_ethernet_frame(ctr, cto_payload);

        {
            let mut w = self.writer.lock().await;
            match &mut *w {
                TransportWriteHalf::Udp {
                    socket,
                    target_addr,
                } => {
                    socket
                        .send_to(&raw_frame, *target_addr)
                        .await
                        .map_err(crate::xcp::client::XcpError::Io)?;
                }
                TransportWriteHalf::Tcp { writer } => {
                    writer
                        .write_all(&raw_frame)
                        .await
                        .map_err(crate::xcp::client::XcpError::Io)?;
                }
            }
        }

        let mut rx = self.cto_rx.lock().await;
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(XcpPacketKind::Response(data))) => Ok(data),
            Ok(Some(XcpPacketKind::Error { code, .. })) => {
                Err(crate::xcp::client::XcpError::SlaveError(code))
            }
            Ok(Some(_)) => Err(crate::xcp::client::XcpError::UnexpectedPacket),
            Ok(None) => Err(crate::xcp::client::XcpError::Disconnected),
            Err(_) => Err(crate::xcp::client::XcpError::Timeout),
        }
    }

    /// Marks the transport as closed.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

async fn dispatch_frame(
    frame: &crate::xcp::packet::XcpFrame,
    cto_tx: &mpsc::Sender<XcpPacketKind>,
    daq_tx: &mpsc::Sender<IncomingDaqDto>,
    link_stats: &StdMutex<LinkStats>,
) {
    // Observe the CTR of every frame, CTO responses included: the counter is shared across the
    // connection, so skipping CTO frames would make each command response look like a lost
    // datagram.
    let link_generation = match link_stats.lock() {
        Ok(mut stats) => {
            stats.observe_ctr(frame.counter);
            stats.generation
        }
        Err(_) => 0,
    };

    if let Some(kind) = frame.classify() {
        match kind {
            XcpPacketKind::Response(_) | XcpPacketKind::Error { .. } => {
                let _ = cto_tx.send(kind).await;
            }
            XcpPacketKind::DaqDto {
                odt_num,
                daq_list_num,
                payload,
            } => {
                // `try_send` keeps the socket reader non-blocking, but a full queue must be
                // counted: an uncounted drop is indistinguishable from the target sending nothing.
                if daq_tx
                    .try_send(IncomingDaqDto {
                        odt_num,
                        daq_list_num,
                        payload,
                        counter: frame.counter,
                        link_generation,
                    })
                    .is_err()
                {
                    if let Ok(mut stats) = link_stats.lock() {
                        stats.note_channel_overflow(1);
                    }
                }
            }
            XcpPacketKind::Event(_) | XcpPacketKind::Service(_) => {}
        }
    }
}
