// SPDX-License-Identifier: GPL-2.0-only
//! High-level XCP Master Client orchestrating connection, automatic A2L upload via
//! `GET_ID (IDT_ASAM_UPLOAD = 4)` + `UPLOAD`, DAQ configuration, target clock sync,
//! and concurrent live calibration (`SHORT_UPLOAD` / `SHORT_DOWNLOAD` / `UPLOAD` / `DOWNLOAD`).

use crate::a2l::model::Characteristic;
use crate::config_store::TransportProtocol;
use crate::xcp::clock_sync::TargetClockSync;
use crate::xcp::daq_planner::DaqPlanner;
use crate::xcp::packet::{cto, IDT_ASAM_UPLOAD};
use crate::xcp::transport::{IncomingDaqDto, XcpTransport};
use chrono::Utc;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;

/// Errors returned by [`XcpClient`] operations.
#[derive(Debug, Error)]
pub enum XcpError {
    #[error("Network I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("XCP slave returned error code 0x{0:02X}")]
    SlaveError(u8),
    #[error("XCP command timed out waiting for target response")]
    Timeout,
    #[error("XCP transport disconnected")]
    Disconnected,
    #[error("Unexpected XCP packet type")]
    UnexpectedPacket,
    #[error("A2L upload failed or returned empty payload: {0}")]
    A2lUpload(String),
    #[error("Calibration readback verification mismatch (wrote {expected:?}, read {actual:?})")]
    ReadbackMismatch { expected: Vec<u8>, actual: Vec<u8> },
    #[error("Refusing to transfer {requested} bytes: above the {limit}-byte safety limit")]
    TransferTooLarge { requested: usize, limit: usize },
}

/// Where the A2L text handed to the parser came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2lSource {
    /// Uploaded from the target via `GET_ID(IDT_ASAM_UPLOAD)` + `UPLOAD`.
    TargetUpload,
    /// Read from the configured local fallback file because the upload failed or was empty.
    LocalFile(std::path::PathBuf),
}

impl std::fmt::Display for A2lSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            A2lSource::TargetUpload => write!(f, "target upload"),
            A2lSource::LocalFile(p) => write!(f, "local file {}", p.display()),
        }
    }
}

/// Upper bound for any single chunked `UPLOAD` transfer (A2L text or memory read). The length
/// comes from the target, so it must never be allowed to size an allocation unchecked.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// High-level XCP Master Client.
#[derive(Clone)]
pub struct XcpClient {
    transport: Arc<XcpTransport>,
    timeout: Duration,
    max_cto: usize,
    max_dto: usize,
}

impl XcpClient {
    /// Connects to the XCP slave at `addr` over `protocol`, issues `CONNECT(0)`,
    /// queries communication mode and initial `GET_DAQ_CLOCK`, and returns the client,
    /// initial [`TargetClockSync`], and the incoming DAQ DTO stream receiver.
    pub async fn connect(
        addr: SocketAddr,
        protocol: TransportProtocol,
        timeout: Duration,
        prefer_target_utc: bool,
    ) -> Result<(Self, TargetClockSync, mpsc::Receiver<IncomingDaqDto>), XcpError> {
        let (transport, daq_rx) = XcpTransport::connect(addr, protocol).await?;
        let res = transport.send_command(&cto::connect(0x00), timeout).await?;

        // Standard CONNECT response: [resource, comm_mode_basic, max_cto, max_dto_le_0, max_dto_le_1, ...]
        let max_cto = res.get(2).copied().unwrap_or(64).max(8) as usize;
        let max_dto = if res.len() >= 5 {
            u16::from_le_bytes([res[3], res[4]]).max(8) as usize
        } else {
            256
        };

        let client = Self {
            transport,
            timeout,
            max_cto,
            max_dto,
        };

        let mut clock_sync = TargetClockSync::new(prefer_target_utc);
        let _ = client.sync_target_clock(&mut clock_sync).await;

        Ok((client, clock_sync, daq_rx))
    }

    /// Queries `GET_DAQ_CLOCK (0xDC)` and updates `clock_sync`.
    pub async fn sync_target_clock(
        &self,
        clock_sync: &mut TargetClockSync,
    ) -> Result<(), XcpError> {
        let res = self
            .transport
            .send_command(&cto::get_daq_clock(), self.timeout)
            .await?;
        let host_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        clock_sync.update_from_get_daq_clock_response(&res, host_ns);
        Ok(())
    }

    /// Fetches the A2L text: `GET_ID(IDT_ASAM_UPLOAD = 4)` + chunked `UPLOAD` from the target
    /// first; only when that fails or yields nothing is `local_fallback_path` read. The returned
    /// [`A2lSource`] says which one won so the UI can show it.
    pub async fn fetch_a2l(
        &self,
        local_fallback_path: Option<&str>,
    ) -> Result<(String, A2lSource), XcpError> {
        let upload_error = match self.upload_a2l_from_target().await {
            Ok(text) => return Ok((text, A2lSource::TargetUpload)),
            Err(e) => e,
        };

        if let Some(path) = local_fallback_path.map(str::trim).filter(|p| !p.is_empty()) {
            match std::fs::read_to_string(path) {
                Ok(content) if !content.trim().is_empty() => {
                    return Ok((content, A2lSource::LocalFile(path.into())));
                }
                Ok(_) => {
                    return Err(XcpError::A2lUpload(format!(
                        "{upload_error}; local fallback '{path}' is empty"
                    )));
                }
                Err(io) => {
                    return Err(XcpError::A2lUpload(format!(
                        "{upload_error}; local fallback '{path}' unreadable: {io}"
                    )));
                }
            }
        }
        Err(upload_error)
    }

    /// `GET_ID(IDT_ASAM_UPLOAD)` followed by a chunked `UPLOAD` of the announced length.
    async fn upload_a2l_from_target(&self) -> Result<String, XcpError> {
        let res = self
            .transport
            .send_command(&cto::get_id(IDT_ASAM_UPLOAD), self.timeout)
            .await?;
        if res.len() < 7 {
            return Err(XcpError::A2lUpload(format!(
                "GET_ID response too short ({} bytes)",
                res.len()
            )));
        }
        let a2l_len = u32::from_le_bytes([res[3], res[4], res[5], res[6]]) as usize;
        if a2l_len > MAX_UPLOAD_BYTES {
            return Err(XcpError::TransferTooLarge {
                requested: a2l_len,
                limit: MAX_UPLOAD_BYTES,
            });
        }
        if a2l_len == 0 {
            return Err(XcpError::A2lUpload(
                "target reports a zero-length A2L".to_string(),
            ));
        }
        let raw_bytes = self.upload_from_current_mta(a2l_len).await?;
        let text = String::from_utf8_lossy(&raw_bytes).into_owned();
        if text.trim().is_empty() {
            return Err(XcpError::A2lUpload(
                "target uploaded an empty A2L".to_string(),
            ));
        }
        Ok(text)
    }

    /// Uploads `total_len` bytes starting from the target's current MTA using chunked `UPLOAD`.
    pub async fn upload_from_current_mta(&self, total_len: usize) -> Result<Vec<u8>, XcpError> {
        if total_len > MAX_UPLOAD_BYTES {
            return Err(XcpError::TransferTooLarge {
                requested: total_len,
                limit: MAX_UPLOAD_BYTES,
            });
        }
        let mut buffer = Vec::with_capacity(total_len);
        let max_chunk = (self.max_cto.saturating_sub(1)).clamp(1, 255);

        while buffer.len() < total_len {
            let remaining = total_len - buffer.len();
            let req_len = remaining.min(max_chunk) as u8;
            let chunk = self
                .transport
                .send_command(&cto::upload(req_len), self.timeout)
                .await?;
            let take_len = chunk.len().min(req_len as usize);
            if take_len == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..take_len]);
        }

        Ok(buffer)
    }

    /// Reads raw memory from the target at `(addr, addr_ext)` of `byte_len` bytes.
    /// Uses `SHORT_UPLOAD` for small transfers or `SET_MTA` + `UPLOAD` for larger transfers.
    pub async fn read_memory(
        &self,
        addr: u32,
        addr_ext: u8,
        byte_len: usize,
    ) -> Result<Vec<u8>, XcpError> {
        let max_short = self.max_cto.saturating_sub(1).min(255);
        if byte_len <= max_short && byte_len > 0 {
            let res = self
                .transport
                .send_command(
                    &cto::short_upload(byte_len as u8, addr, addr_ext),
                    self.timeout,
                )
                .await?;
            Ok(res[..res.len().min(byte_len)].to_vec())
        } else {
            self.transport
                .send_command(&cto::set_mta(addr, addr_ext), self.timeout)
                .await?;
            self.upload_from_current_mta(byte_len).await
        }
    }

    /// Writes raw bytes to the target at `(addr, addr_ext)` using `SHORT_DOWNLOAD` (for small writes)
    /// or `SET_MTA` + `DOWNLOAD` (for larger writes). Optionally verifies by reading back.
    pub async fn write_memory(
        &self,
        addr: u32,
        addr_ext: u8,
        data: &[u8],
        verify_readback: bool,
    ) -> Result<(), XcpError> {
        let max_short_data = self.max_cto.saturating_sub(8).min(255);
        if data.len() <= max_short_data && !data.is_empty() {
            self.transport
                .send_command(&cto::short_download(addr, addr_ext, data), self.timeout)
                .await?;
        } else {
            self.transport
                .send_command(&cto::set_mta(addr, addr_ext), self.timeout)
                .await?;
            let max_dl_chunk = self.max_cto.saturating_sub(2).clamp(1, 255);
            let mut offset = 0;
            while offset < data.len() {
                let end = (offset + max_dl_chunk).min(data.len());
                self.transport
                    .send_command(&cto::download(&data[offset..end]), self.timeout)
                    .await?;
                offset = end;
            }
        }

        if verify_readback {
            let readback = self.read_memory(addr, addr_ext, data.len()).await?;
            if readback != data {
                return Err(XcpError::ReadbackMismatch {
                    expected: data.to_vec(),
                    actual: readback,
                });
            }
        }

        Ok(())
    }

    /// Reads the raw bytes of an A2L [`Characteristic`].
    pub async fn read_characteristic(&self, ch: &Characteristic) -> Result<Vec<u8>, XcpError> {
        self.read_memory(ch.address, ch.address_extension, ch.total_byte_size())
            .await
    }

    /// Switches active calibration page on the target via `SET_CAL_PAGE (0xEB)`.
    pub async fn set_calibration_page(&self, segment: u8, page: u8) -> Result<(), XcpError> {
        // Mode 0x03 = ECU (0x01) | XCP (0x02)
        self.transport
            .send_command(&cto::set_cal_page(0x03, segment, page), self.timeout)
            .await?;
        Ok(())
    }

    /// Configures and starts all DAQ lists planned in `planner` on the XCP slave.
    pub async fn configure_and_start_daq(&self, planner: &DaqPlanner) -> Result<(), XcpError> {
        self.transport
            .send_command(&cto::free_daq(), self.timeout)
            .await?;

        if planner.lists.is_empty() {
            return Ok(());
        }

        self.transport
            .send_command(&cto::alloc_daq(planner.lists.len() as u16), self.timeout)
            .await?;

        for list in &planner.lists {
            self.transport
                .send_command(
                    &cto::alloc_odt(list.daq_list_num, list.odts.len() as u8),
                    self.timeout,
                )
                .await?;
        }

        for list in &planner.lists {
            for odt in &list.odts {
                self.transport
                    .send_command(
                        &cto::alloc_odt_entry(
                            list.daq_list_num,
                            odt.odt_num,
                            odt.entries.len() as u8,
                        ),
                        self.timeout,
                    )
                    .await?;
            }
        }

        for list in &planner.lists {
            for odt in &list.odts {
                self.transport
                    .send_command(
                        &cto::set_daq_ptr(list.daq_list_num, odt.odt_num, 0),
                        self.timeout,
                    )
                    .await?;
                for entry in &odt.entries {
                    self.transport
                        .send_command(
                            &cto::write_daq(
                                0xFF,
                                entry.byte_size,
                                entry.address_extension,
                                entry.address,
                            ),
                            self.timeout,
                        )
                        .await?;
                }
            }
            // Mode 0x10 = TIMESTAMP enabled
            self.transport
                .send_command(
                    &cto::set_daq_list_mode(0x10, list.daq_list_num, list.event_channel_id, 1, 0),
                    self.timeout,
                )
                .await?;
            // Mode 0x02 = select DAQ list for synchronized start
            self.transport
                .send_command(
                    &cto::start_stop_daq_list(0x02, list.daq_list_num),
                    self.timeout,
                )
                .await?;
        }

        // Mode 0x01 = start selected DAQ lists synchronously
        self.transport
            .send_command(&cto::start_stop_synch(0x01), self.timeout)
            .await?;

        Ok(())
    }

    /// Stops all running DAQ lists on the target (`START_STOP_SYNCH(0x00)`).
    pub async fn stop_daq(&self) -> Result<(), XcpError> {
        self.transport
            .send_command(&cto::start_stop_synch(0x00), self.timeout)
            .await?;
        Ok(())
    }

    /// Gracefully disconnects from the XCP target (`DISCONNECT(0xFE)`) and closes the socket.
    pub async fn disconnect(&self) -> Result<(), XcpError> {
        let _ = self
            .transport
            .send_command(&cto::disconnect(), self.timeout)
            .await;
        self.transport.close();
        Ok(())
    }

    /// Returns a snapshot of link-integrity counters (CTR gaps, reordering, duplicates and DAQ
    /// queue overflow drops) observed across every frame received on this connection.
    pub fn link_stats(&self) -> crate::xcp::link_stats::LinkStats {
        self.transport.link_stats()
    }

    /// Returns the negotiated `MAX_DTO` size.
    pub fn max_dto(&self) -> usize {
        self.max_dto
    }
}
