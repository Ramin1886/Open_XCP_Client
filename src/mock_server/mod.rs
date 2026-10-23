// SPDX-License-Identifier: GPL-2.0-only
//! Built-in asynchronous Mock `xcplite` 2.2.2 XCP on Ethernet (UDP and TCP) server.
//!
//! Emulates an embedded QNX 7.1 ADAS target with dynamic A2L upload (`GET_ID` mode 4),
//! simulated RAM for complex C/C++ structs and calibration parameters, target UTC clock
//! (`GET_DAQ_CLOCK`), and multi-rate DAQ DTO streaming.

pub mod sample_adas_a2l;

use crate::xcp::packet::{
    encode_xcp_ethernet_frame, unpack_xcp_ethernet_frames, XcpCommand, IDT_ASAM_NAME,
    IDT_ASAM_UPLOAD,
};
use chrono::Utc;
use sample_adas_a2l::sample_adas_a2l_text;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
struct MockOdtEntry {
    addr: u32,
    size: u8,
}

#[derive(Debug, Clone, Default)]
struct MockOdt {
    entries: Vec<MockOdtEntry>,
}

#[derive(Debug, Clone, Default)]
struct MockDaqList {
    event_channel: u16,
    selected: bool,
    odts: Vec<MockOdt>,
}

struct MockSlaveState {
    memory: BTreeMap<u32, u8>,
    a2l_bytes: Vec<u8>,
    mta_addr: u32,
    mta_is_a2l: bool,
    cal_page: u8,
    daq_lists: Vec<MockDaqList>,
    daq_ptr: (u16, u8, u8),
    daq_running: bool,
    tx_counter: u16,
    sim_step: u64,
}

impl MockSlaveState {
    fn new() -> Self {
        let mut s = Self {
            memory: BTreeMap::new(),
            a2l_bytes: sample_adas_a2l_text().as_bytes().to_vec(),
            mta_addr: 0,
            mta_is_a2l: false,
            cal_page: 0,
            daq_lists: Vec::new(),
            daq_ptr: (0, 0, 0),
            daq_running: false,
            tx_counter: 0,
            sim_step: 0,
        };
        s.init_default_memory();
        s
    }

    fn init_default_memory(&mut self) {
        // Initialize Perception.RadarFront @ 0x1000
        self.write_u32(0x1000, 1); // frame_counter
        self.write_f32(0x1004, 0.75); // min_confidence
                                      // targets[0] @ 0x1008
        self.write_f32(0x1008, 42.5); // distance_m
        self.write_f32(0x100C, -3.2); // velocity_mps
        self.write_f32(0x1010, 0.45); // lat_offset_m
        self.write_f32(0x1014, 18.0); // snr_db
                                      // targets[1] @ 0x1018
        self.write_f32(0x1018, 78.0);
        self.write_f32(0x101C, 1.5);
        self.write_f32(0x1020, -1.8);
        self.write_f32(0x1024, 14.5);

        // EgoMotion @ 0x2000
        self.write_f32(0x2000, 22.5); // velocity_x
        self.write_f64(0x2008, 1.25); // yaw_rate_deg_s
        self.write_f32(0x2010, 0.05); // covariance_diag[0]
        self.write_f32(0x2014, 0.08); // covariance_diag[1]
        self.write_f32(0x2018, 0.02); // covariance_diag[2]
        self.write_bytes(0x2020, &[34]); // QnxCpuLoadPercent

        // Calibration defaults @ 0x3000
        self.write_f32(0x3000, 2.5); // kp_gain
        self.write_f64(0x3008, 25.0); // max_steering_deg
        self.write_bytes(0x3010, &32u16.to_le_bytes()); // max_tracked_objects = 32
        self.write_bytes(0x3012, &[1]); // enable_aeb = 1
        self.write_bytes(0x3014, &[2]); // operating_mode = 2 (ACTIVE_LKA)
                                        // speed_lookup_curve[4] @ 0x3020
        self.write_f32(0x3020, 1.0);
        self.write_f32(0x3024, 1.5);
        self.write_f32(0x3028, 2.0);
        self.write_f32(0x302C, 2.5);
    }

    fn write_bytes(&mut self, addr: u32, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.memory.insert(addr.wrapping_add(i as u32), b);
        }
    }

    fn read_bytes(&self, addr: u32, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                self.memory
                    .get(&addr.wrapping_add(i as u32))
                    .copied()
                    .unwrap_or(0)
            })
            .collect()
    }

    fn write_u32(&mut self, addr: u32, v: u32) {
        self.write_bytes(addr, &v.to_le_bytes());
    }

    fn write_f32(&mut self, addr: u32, v: f32) {
        self.write_bytes(addr, &v.to_le_bytes());
    }

    fn write_f64(&mut self, addr: u32, v: f64) {
        self.write_bytes(addr, &v.to_le_bytes());
    }

    fn step_simulation(&mut self) {
        self.sim_step = self.sim_step.wrapping_add(1);
        let t = self.sim_step as f32 * 0.05;
        self.write_u32(0x1000, self.sim_step as u32);
        self.write_f32(0x1008, 40.0 + (t.sin() * 5.0));
        self.write_f32(0x100C, -2.5 + (t.cos() * 0.5));
        self.write_f32(0x2000, 22.0 + (t.sin() * 1.5));
        self.write_f64(0x2008, (t as f64).cos() * 2.0);
    }

    fn handle_cto(&mut self, cmd: &[u8]) -> Vec<u8> {
        if cmd.is_empty() {
            return vec![0xFE, 0x20];
        }
        let pid = cmd[0];
        match pid {
            x if x == XcpCommand::Connect as u8 => {
                // [0xFF, resource=0x05(CAL|DAQ), comm_mode=0x00, max_cto=64, max_dto_le=256, proto_ver=1, trans_ver=1]
                vec![0xFF, 0x05, 0x00, 64, 0x00, 0x01, 0x01, 0x01]
            }
            x if x == XcpCommand::Disconnect as u8 => {
                self.daq_running = false;
                vec![0xFF]
            }
            x if x == XcpCommand::GetStatus as u8 => {
                vec![0xFF, 0x00, 0x00, 0x00, 0x00, 0x00]
            }
            x if x == XcpCommand::GetCommModeInfo as u8 => {
                vec![0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
            }
            x if x == XcpCommand::GetId as u8 => {
                let mode = cmd.get(1).copied().unwrap_or(0);
                if mode == IDT_ASAM_UPLOAD || mode == IDT_ASAM_NAME {
                    self.mta_is_a2l = true;
                    self.mta_addr = 0;
                    let len = self.a2l_bytes.len() as u32;
                    let mut res = vec![0xFF, 0x00, 0x00, 0x00];
                    res.extend_from_slice(&len.to_le_bytes());
                    res
                } else {
                    vec![0xFF, 0x00, 0x00, 0x00, 0, 0, 0, 0]
                }
            }
            x if x == XcpCommand::SetMta as u8 => {
                if cmd.len() >= 8 {
                    self.mta_addr = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
                    self.mta_is_a2l = false;
                    vec![0xFF]
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::Upload as u8 => {
                let count = cmd.get(1).copied().unwrap_or(0) as usize;
                let mut res = Vec::with_capacity(1 + count);
                res.push(0xFF);
                if self.mta_is_a2l {
                    let start = (self.mta_addr as usize).min(self.a2l_bytes.len());
                    let end = (start + count).min(self.a2l_bytes.len());
                    res.extend_from_slice(&self.a2l_bytes[start..end]);
                    self.mta_addr = end as u32;
                } else {
                    let bytes = self.read_bytes(self.mta_addr, count);
                    self.mta_addr = self.mta_addr.wrapping_add(count as u32);
                    res.extend_from_slice(&bytes);
                }
                res
            }
            x if x == XcpCommand::ShortUpload as u8 => {
                if cmd.len() >= 8 {
                    let count = cmd[1] as usize;
                    let addr = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
                    self.mta_is_a2l = false;
                    self.mta_addr = addr.wrapping_add(count as u32);
                    let mut res = Vec::with_capacity(1 + count);
                    res.push(0xFF);
                    res.extend_from_slice(&self.read_bytes(addr, count));
                    res
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::Download as u8 => {
                if cmd.len() >= 2 {
                    let count = cmd[1] as usize;
                    let data = &cmd[2..cmd.len().min(2 + count)];
                    self.write_bytes(self.mta_addr, data);
                    self.mta_addr = self.mta_addr.wrapping_add(data.len() as u32);
                    vec![0xFF]
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::ShortDownload as u8 => {
                if cmd.len() >= 8 {
                    let count = cmd[1] as usize;
                    let addr = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
                    let data = &cmd[8..cmd.len().min(8 + count)];
                    self.write_bytes(addr, data);
                    self.mta_addr = addr.wrapping_add(data.len() as u32);
                    vec![0xFF]
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::SetCalPage as u8 => {
                self.cal_page = cmd.get(3).copied().unwrap_or(0);
                vec![0xFF]
            }
            x if x == XcpCommand::GetCalPage as u8 => {
                vec![0xFF, 0x00, 0x00, self.cal_page]
            }
            x if x == XcpCommand::GetDaqClock as u8 => {
                let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
                let mut res = vec![0xFF, 0x00, 0x00, 0x02];
                res.extend_from_slice(&now_ns.to_le_bytes());
                res
            }
            x if x == XcpCommand::FreeDaq as u8 => {
                self.daq_running = false;
                self.daq_lists.clear();
                vec![0xFF]
            }
            x if x == XcpCommand::AllocDaq as u8 => {
                if cmd.len() >= 4 {
                    let count = u16::from_le_bytes([cmd[2], cmd[3]]) as usize;
                    self.daq_lists = vec![MockDaqList::default(); count];
                    vec![0xFF]
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::AllocOdt as u8 => {
                if cmd.len() >= 5 {
                    let daq_idx = u16::from_le_bytes([cmd[2], cmd[3]]) as usize;
                    let odt_count = cmd[4] as usize;
                    if let Some(list) = self.daq_lists.get_mut(daq_idx) {
                        list.odts = vec![MockOdt::default(); odt_count];
                        return vec![0xFF];
                    }
                }
                vec![0xFE, 0x22]
            }
            x if x == XcpCommand::AllocOdtEntry as u8 => {
                if cmd.len() >= 6 {
                    let daq_idx = u16::from_le_bytes([cmd[2], cmd[3]]) as usize;
                    let odt_idx = cmd[4] as usize;
                    let entry_count = cmd[5] as usize;
                    if let Some(list) = self.daq_lists.get_mut(daq_idx) {
                        if let Some(odt) = list.odts.get_mut(odt_idx) {
                            odt.entries = vec![MockOdtEntry::default(); entry_count];
                            return vec![0xFF];
                        }
                    }
                }
                vec![0xFE, 0x22]
            }
            x if x == XcpCommand::SetDaqPtr as u8 => {
                if cmd.len() >= 6 {
                    let daq_idx = u16::from_le_bytes([cmd[2], cmd[3]]);
                    let odt_idx = cmd[4];
                    let entry_idx = cmd[5];
                    self.daq_ptr = (daq_idx, odt_idx, entry_idx);
                    vec![0xFF]
                } else {
                    vec![0xFE, 0x22]
                }
            }
            x if x == XcpCommand::WriteDaq as u8 => {
                if cmd.len() >= 8 {
                    let size = cmd[2];
                    let addr = u32::from_le_bytes([cmd[4], cmd[5], cmd[6], cmd[7]]);
                    let (d, o, e) = self.daq_ptr;
                    if let Some(list) = self.daq_lists.get_mut(d as usize) {
                        if let Some(odt) = list.odts.get_mut(o as usize) {
                            if let Some(entry) = odt.entries.get_mut(e as usize) {
                                entry.addr = addr;
                                entry.size = size;
                                self.daq_ptr.2 = self.daq_ptr.2.wrapping_add(1);
                                return vec![0xFF];
                            }
                        }
                    }
                }
                vec![0xFE, 0x22]
            }
            x if x == XcpCommand::SetDaqListMode as u8 => {
                if cmd.len() >= 6 {
                    let daq_idx = u16::from_le_bytes([cmd[2], cmd[3]]) as usize;
                    let ev_ch = u16::from_le_bytes([cmd[4], cmd[5]]);
                    if let Some(list) = self.daq_lists.get_mut(daq_idx) {
                        list.event_channel = ev_ch;
                        return vec![0xFF];
                    }
                }
                vec![0xFE, 0x22]
            }
            x if x == XcpCommand::StartStopDaqList as u8 => {
                if cmd.len() >= 4 {
                    let mode = cmd[1];
                    let daq_idx = u16::from_le_bytes([cmd[2], cmd[3]]) as usize;
                    if let Some(list) = self.daq_lists.get_mut(daq_idx) {
                        list.selected = mode != 0;
                        return vec![0xFF, 0x00];
                    }
                }
                vec![0xFE, 0x22]
            }
            x if x == XcpCommand::StartStopSynch as u8 => {
                let mode = cmd.get(1).copied().unwrap_or(0);
                self.daq_running = mode == 0x01;
                vec![0xFF]
            }
            _ => vec![0xFE, 0x20],
        }
    }

    fn build_daq_frames_for_tick(&mut self, tick_count: u64) -> Vec<Vec<u8>> {
        if !self.daq_running {
            return Vec::new();
        }
        self.step_simulation();
        let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        let low32_ts = (now_ns & 0xFFFF_FFFF) as u32;

        let mut frames = Vec::new();
        for (daq_idx, list) in self.daq_lists.iter().enumerate() {
            if !list.selected {
                continue;
            }
            // Event 0 fires every tick (10ms); Event 1 fires every 5th tick (50ms)
            if list.event_channel == 1 && !tick_count.is_multiple_of(5) {
                continue;
            }
            for (odt_idx, odt) in list.odts.iter().enumerate() {
                let mut dto = Vec::with_capacity(64);
                dto.push(odt_idx as u8);
                dto.push(daq_idx as u8);
                if odt_idx == 0 {
                    dto.extend_from_slice(&low32_ts.to_le_bytes());
                }
                for entry in &odt.entries {
                    dto.extend_from_slice(&self.read_bytes(entry.addr, entry.size as usize));
                }
                let ctr = self.tx_counter;
                self.tx_counter = self.tx_counter.wrapping_add(1);
                frames.push(encode_xcp_ethernet_frame(ctr, &dto));
            }
        }
        frames
    }
}

/// Handle to a running Mock `xcplite` 2.2.2 Server instance (UDP and TCP).
pub struct MockXcpServer {
    pub udp_addr: SocketAddr,
    pub tcp_addr: SocketAddr,
    stop_flag: Arc<AtomicBool>,
}

impl MockXcpServer {
    /// Starts a mock `xcplite` 2.2.2 server bound to `127.0.0.1:port` (or `0` for OS-assigned ephemeral ports).
    pub async fn start(bind_port: u16) -> std::io::Result<Self> {
        let udp_sock = Arc::new(UdpSocket::bind(("127.0.0.1", bind_port)).await?);
        let udp_addr = udp_sock.local_addr()?;

        let tcp_listener = TcpListener::bind(("127.0.0.1", bind_port)).await?;
        let tcp_addr = tcp_listener.local_addr()?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(MockSlaveState::new()));

        // Spawn UDP handler + DAQ ticker
        {
            let udp_clone = Arc::clone(&udp_sock);
            let state_clone = Arc::clone(&state);
            let stop_clone = Arc::clone(&stop_flag);
            let last_client: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));

            let udp_daq_sock = Arc::clone(&udp_sock);
            let state_daq = Arc::clone(&state);
            let stop_daq = Arc::clone(&stop_flag);
            let last_client_daq = Arc::clone(&last_client);

            tokio::spawn(async move {
                let mut tick = 0u64;
                let mut interval = tokio::time::interval(Duration::from_millis(10));
                while !stop_daq.load(Ordering::Relaxed) {
                    interval.tick().await;
                    tick = tick.wrapping_add(1);
                    let target_peer = *last_client_daq.lock().await;
                    if let Some(peer) = target_peer {
                        let frames = state_daq.lock().await.build_daq_frames_for_tick(tick);
                        for f in frames {
                            let _ = udp_daq_sock.send_to(&f, peer).await;
                        }
                    }
                }
            });

            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                while !stop_clone.load(Ordering::Relaxed) {
                    if let Ok(Ok((n, peer))) = tokio::time::timeout(
                        Duration::from_millis(50),
                        udp_clone.recv_from(&mut buf),
                    )
                    .await
                    {
                        *last_client.lock().await = Some(peer);
                        let (frames, _) = unpack_xcp_ethernet_frames(&buf[..n]);
                        for frame in frames {
                            let mut st = state_clone.lock().await;
                            let reply = st.handle_cto(&frame.payload);
                            let ctr = st.tx_counter;
                            st.tx_counter = st.tx_counter.wrapping_add(1);
                            let wire = encode_xcp_ethernet_frame(ctr, &reply);
                            drop(st);
                            let _ = udp_clone.send_to(&wire, peer).await;
                        }
                    }
                }
            });
        }

        // Spawn TCP listener + per-connection CTO & DAQ loop
        {
            let stop_tcp = Arc::clone(&stop_flag);
            tokio::spawn(async move {
                while !stop_tcp.load(Ordering::Relaxed) {
                    if let Ok(Ok((stream, _))) =
                        tokio::time::timeout(Duration::from_millis(50), tcp_listener.accept()).await
                    {
                        let _ = stream.set_nodelay(true);
                        let (mut reader, writer) = stream.into_split();
                        let writer = Arc::new(Mutex::new(writer));
                        let conn_state = Arc::new(Mutex::new(MockSlaveState::new()));
                        let conn_stop = Arc::clone(&stop_tcp);

                        let writer_daq = Arc::clone(&writer);
                        let state_daq = Arc::clone(&conn_state);
                        let stop_daq = Arc::clone(&conn_stop);
                        tokio::spawn(async move {
                            let mut tick = 0u64;
                            let mut interval = tokio::time::interval(Duration::from_millis(10));
                            while !stop_daq.load(Ordering::Relaxed) {
                                interval.tick().await;
                                tick = tick.wrapping_add(1);
                                let frames = state_daq.lock().await.build_daq_frames_for_tick(tick);
                                if !frames.is_empty() {
                                    let mut w = writer_daq.lock().await;
                                    for f in frames {
                                        if w.write_all(&f).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        });

                        tokio::spawn(async move {
                            let mut stream_buf = Vec::with_capacity(8192);
                            let mut tmp = [0u8; 2048];
                            while !conn_stop.load(Ordering::Relaxed) {
                                match reader.read(&mut tmp).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        stream_buf.extend_from_slice(&tmp[..n]);
                                        let (frames, consumed) =
                                            unpack_xcp_ethernet_frames(&stream_buf);
                                        if consumed > 0 {
                                            stream_buf.drain(0..consumed);
                                        }
                                        for frame in frames {
                                            let mut st = conn_state.lock().await;
                                            let reply = st.handle_cto(&frame.payload);
                                            let ctr = st.tx_counter;
                                            st.tx_counter = st.tx_counter.wrapping_add(1);
                                            let wire = encode_xcp_ethernet_frame(ctr, &reply);
                                            drop(st);
                                            let mut w = writer.lock().await;
                                            if w.write_all(&wire).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                }
            });
        }

        Ok(Self {
            udp_addr,
            tcp_addr,
            stop_flag,
        })
    }

    /// Stops the mock server background tasks.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for MockXcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}
