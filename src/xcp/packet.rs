// SPDX-License-Identifier: GPL-2.0-only
//! XCP on Ethernet framing (4-byte header: `len: u16 LE`, `ctr: u16 LE`) and
//! CTO/DTO command builders and parsers compatible with Vector `xcplite` 2.2.2.

/// XCP CTO Command codes (`CC_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum XcpCommand {
    Connect = 0xFF,
    Disconnect = 0xFE,
    GetStatus = 0xFD,
    Synch = 0xFC,
    GetCommModeInfo = 0xFB,
    GetId = 0xFA,
    SetRequest = 0xF9,
    SetMta = 0xF6,
    Upload = 0xF5,
    ShortUpload = 0xF4,
    Download = 0xF0,
    ShortDownload = 0xED,
    SetCalPage = 0xEB,
    GetCalPage = 0xEA,
    SetDaqPtr = 0xE2,
    WriteDaq = 0xE1,
    SetDaqListMode = 0xE0,
    StartStopDaqList = 0xDE,
    StartStopSynch = 0xDD,
    GetDaqClock = 0xDC,
    FreeDaq = 0xD6,
    AllocDaq = 0xD5,
    AllocOdt = 0xD4,
    AllocOdtEntry = 0xD3,
}

/// Identification type for `GET_ID (0xFA)`.
pub const IDT_ASCII: u8 = 0;
pub const IDT_ASAM_NAME: u8 = 1;
pub const IDT_ASAM_PATH: u8 = 2;
pub const IDT_ASAM_URL: u8 = 3;
/// Vector `xcplite` 2.2.2 direct A2L file upload mode (`GET_ID` mode 4).
pub const IDT_ASAM_UPLOAD: u8 = 4;

/// Classification of an incoming XCP slave-to-master packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XcpPacketKind {
    /// `0xFF`: Positive command response (`RES`), containing payload bytes after `0xFF`.
    Response(Vec<u8>),
    /// `0xFE`: Negative command response (`ERR`), with error code and optional detail bytes.
    Error { code: u8, data: Vec<u8> },
    /// `0xFD`: Asynchronous Event packet (`EV`).
    Event(Vec<u8>),
    /// `0xFC`: Service packet (`SERV`).
    Service(Vec<u8>),
    /// `0x00..=0xFB`: Data Acquisition DTO packet (`DAQ`).
    DaqDto {
        odt_num: u8,
        daq_list_num: u8,
        payload: Vec<u8>,
    },
}

/// A decoded XCP on Ethernet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XcpFrame {
    pub counter: u16,
    pub payload: Vec<u8>,
}

impl XcpFrame {
    /// Classifies a slave-to-master XCP frame into a [`XcpPacketKind`].
    pub fn classify(&self) -> Option<XcpPacketKind> {
        let first = *self.payload.first()?;
        match first {
            0xFF => Some(XcpPacketKind::Response(self.payload[1..].to_vec())),
            0xFE => {
                let code = self.payload.get(1).copied().unwrap_or(0);
                let data = if self.payload.len() > 2 {
                    self.payload[2..].to_vec()
                } else {
                    Vec::new()
                };
                Some(XcpPacketKind::Error { code, data })
            }
            0xFD => Some(XcpPacketKind::Event(self.payload[1..].to_vec())),
            0xFC => Some(XcpPacketKind::Service(self.payload[1..].to_vec())),
            odt_num => {
                let daq_list_num = self.payload.get(1).copied().unwrap_or(0);
                let payload = if self.payload.len() > 2 {
                    self.payload[2..].to_vec()
                } else {
                    Vec::new()
                };
                Some(XcpPacketKind::DaqDto {
                    odt_num,
                    daq_list_num,
                    payload,
                })
            }
        }
    }
}

/// Encodes an XCP payload with the 4-byte XCP-on-Ethernet header (`len: u16 LE`, `ctr: u16 LE`).
pub fn encode_xcp_ethernet_frame(counter: u16, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&counter.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Unpacks one or more concatenated XCP-on-Ethernet frames from a datagram or stream slice.
/// Returns `(decoded_frames, bytes_consumed)`.
pub fn unpack_xcp_ethernet_frames(buf: &[u8]) -> (Vec<XcpFrame>, usize) {
    let mut frames = Vec::new();
    let mut offset = 0usize;
    while offset + 4 <= buf.len() {
        let len = u16::from_le_bytes([buf[offset], buf[offset + 1]]) as usize;
        let ctr = u16::from_le_bytes([buf[offset + 2], buf[offset + 3]]);
        if len == 0 {
            offset += 4;
            continue;
        }
        if offset + 4 + len > buf.len() {
            break;
        }
        let payload = buf[offset + 4..offset + 4 + len].to_vec();
        frames.push(XcpFrame {
            counter: ctr,
            payload,
        });
        offset += 4 + len;
    }
    (frames, offset)
}

/// Helper constructors for standard XCP master CTO command payloads.
pub mod cto {
    use super::XcpCommand;

    pub fn connect(mode: u8) -> Vec<u8> {
        vec![XcpCommand::Connect as u8, mode]
    }

    pub fn disconnect() -> Vec<u8> {
        vec![XcpCommand::Disconnect as u8]
    }

    pub fn get_status() -> Vec<u8> {
        vec![XcpCommand::GetStatus as u8]
    }

    pub fn get_comm_mode_info() -> Vec<u8> {
        vec![XcpCommand::GetCommModeInfo as u8]
    }

    pub fn get_id(id_type: u8) -> Vec<u8> {
        vec![XcpCommand::GetId as u8, id_type]
    }

    pub fn set_mta(addr: u32, addr_ext: u8) -> Vec<u8> {
        let mut p = vec![XcpCommand::SetMta as u8, 0, 0, addr_ext];
        p.extend_from_slice(&addr.to_le_bytes());
        p
    }

    pub fn upload(num_elements: u8) -> Vec<u8> {
        vec![XcpCommand::Upload as u8, num_elements]
    }

    pub fn short_upload(num_elements: u8, addr: u32, addr_ext: u8) -> Vec<u8> {
        let mut p = vec![XcpCommand::ShortUpload as u8, num_elements, 0, addr_ext];
        p.extend_from_slice(&addr.to_le_bytes());
        p
    }

    pub fn download(data: &[u8]) -> Vec<u8> {
        let mut p = Vec::with_capacity(2 + data.len());
        p.push(XcpCommand::Download as u8);
        p.push(data.len() as u8);
        p.extend_from_slice(data);
        p
    }

    pub fn short_download(addr: u32, addr_ext: u8, data: &[u8]) -> Vec<u8> {
        let mut p = Vec::with_capacity(8 + data.len());
        p.push(XcpCommand::ShortDownload as u8);
        p.push(data.len() as u8);
        p.push(0);
        p.push(addr_ext);
        p.extend_from_slice(&addr.to_le_bytes());
        p.extend_from_slice(data);
        p
    }

    pub fn set_cal_page(mode: u8, segment: u8, page: u8) -> Vec<u8> {
        vec![XcpCommand::SetCalPage as u8, mode, segment, page]
    }

    pub fn get_daq_clock() -> Vec<u8> {
        vec![XcpCommand::GetDaqClock as u8]
    }

    pub fn free_daq() -> Vec<u8> {
        vec![XcpCommand::FreeDaq as u8]
    }

    pub fn alloc_daq(daq_count: u16) -> Vec<u8> {
        let mut p = vec![XcpCommand::AllocDaq as u8, 0];
        p.extend_from_slice(&daq_count.to_le_bytes());
        p
    }

    pub fn alloc_odt(daq_list_num: u16, odt_count: u8) -> Vec<u8> {
        let mut p = vec![XcpCommand::AllocOdt as u8, 0];
        p.extend_from_slice(&daq_list_num.to_le_bytes());
        p.push(odt_count);
        p
    }

    pub fn alloc_odt_entry(daq_list_num: u16, odt_num: u8, entry_count: u8) -> Vec<u8> {
        let mut p = vec![XcpCommand::AllocOdtEntry as u8, 0];
        p.extend_from_slice(&daq_list_num.to_le_bytes());
        p.push(odt_num);
        p.push(entry_count);
        p
    }

    pub fn set_daq_ptr(daq_list_num: u16, odt_num: u8, odt_entry_num: u8) -> Vec<u8> {
        let mut p = vec![XcpCommand::SetDaqPtr as u8, 0];
        p.extend_from_slice(&daq_list_num.to_le_bytes());
        p.push(odt_num);
        p.push(odt_entry_num);
        p
    }

    pub fn write_daq(bit_offset: u8, size: u8, addr_ext: u8, addr: u32) -> Vec<u8> {
        let mut p = vec![XcpCommand::WriteDaq as u8, bit_offset, size, addr_ext];
        p.extend_from_slice(&addr.to_le_bytes());
        p
    }

    pub fn set_daq_list_mode(
        mode: u8,
        daq_list_num: u16,
        event_channel_num: u16,
        prescaler: u8,
        priority: u8,
    ) -> Vec<u8> {
        let mut p = vec![XcpCommand::SetDaqListMode as u8, mode];
        p.extend_from_slice(&daq_list_num.to_le_bytes());
        p.extend_from_slice(&event_channel_num.to_le_bytes());
        p.push(prescaler);
        p.push(priority);
        p
    }

    pub fn start_stop_daq_list(mode: u8, daq_list_num: u16) -> Vec<u8> {
        let mut p = vec![XcpCommand::StartStopDaqList as u8, mode];
        p.extend_from_slice(&daq_list_num.to_le_bytes());
        p
    }

    pub fn start_stop_synch(mode: u8) -> Vec<u8> {
        vec![XcpCommand::StartStopSynch as u8, mode]
    }
}
