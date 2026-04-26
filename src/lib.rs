//!
//! S7 PLC Simulator
//! 
//! A simulated S7 PLC with Web Admin API

pub mod memory;
pub mod api;

use std::sync::{Arc, RwLock};
use std::collections::VecDeque;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

pub use memory::{PlcMemory, SharedMemory, create_shared_memory, create_shared_memory_from_config, MemoryArea, DataBlock, DataBlockData, VariableDefinition, DataBlockConfig, PlcConfig};

#[cfg(test)]
mod tests;

/// Shared connection tracker
pub type ConnectionList = Arc<RwLock<Vec<ClientConnection>>>;

/// Information about a connected client
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientConnection {
    pub id: usize,
    pub remote_addr: String,
    pub connected_at: String,
    pub last_activity: String,
    pub requests_count: u64,
    pub framing: String,
    pub state: String,
}

/// Shared in-memory log buffer (last 10000 entries)
pub type LogBuffer = Arc<RwLock<VecDeque<LogEntry>>>;

/// Log entry structure
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

impl LogEntry {
    pub fn new(level: &str, message: &str) -> Self {
        let now = chrono::Local::now();
        Self {
            timestamp: now.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level: level.to_string(),
            message: message.to_string(),
        }
    }
}

/// Create a new shared connection list
pub fn create_connection_list() -> ConnectionList {
    Arc::new(RwLock::new(Vec::new()))
}

/// Create a new log buffer
pub fn create_log_buffer() -> LogBuffer {
    Arc::new(RwLock::new(VecDeque::with_capacity(10000)))
}

/// Add a log entry to the buffer (thread-safe, auto-trims to 10000)
pub fn add_log_entry(log_buffer: &LogBuffer, level: &str, message: &str) {
    if let Ok(mut logs) = log_buffer.write() {
        logs.push_back(LogEntry::new(level, message));
        while logs.len() > 10000 {
            logs.pop_front();
        }
    }
}

static CONNECTION_ID_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

/// Protocol ID
const PROTOCOL_ID: u8 = 0x32;

/// PDU type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PduType {
    AckData = 0x03,
}

/// Function codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionCode {
    SetupCommunication = 0xF0,
    Read = 0x04,
    Write = 0x05,
}

/// Transport size
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum TransportSize {
    Bit = 0x01,
    Byte = 0x02,
    Word = 0x04,
    DWord = 0x06,
}

/// S7 PLC Simulator
pub struct PlcSimulator {
    /// PLC memory
    memory: SharedMemory,
    /// PLC type
    plc_type: String,
    /// Rack and slot
    rack: u8,
    slot: u8,
    /// Active connections tracker
    connections: ConnectionList,
    /// Log buffer for web UI
    log_buffer: LogBuffer,
}

impl PlcSimulator {
    /// Create new PLC simulator
    pub fn new(plc_type: &str, rack: u8, slot: u8, memory: SharedMemory, connections: ConnectionList, log_buffer: LogBuffer) -> Self {
        Self {
            memory,
            plc_type: plc_type.to_string(),
            rack,
            slot,
            connections,
            log_buffer,
        }
    }
    
    /// Handle incoming connection
    /// Read exactly `n` bytes from the stream into buffer.
    /// Returns Ok(Some(bytes)) on success, Ok(None) on EOF.
    async fn read_exact_buf(
        stream: &mut tokio::net::TcpStream,
        buf: &mut Vec<u8>,
        buf_start: &mut usize,
        buf_end: &mut usize,
        n: usize,
    ) -> Result<bool, std::io::Error> {
        // buf is pre-allocated and always zeroed; ensure capacity for n bytes
        if buf.len() < n {
            buf.resize(n, 0);
        }
        // Safety: ensure buf_start <= buf_end (invariant check)
        if *buf_start > *buf_end {
            warn!("[S7] BUG: buf_start={} > buf_end={}, resetting", *buf_start, *buf_end);
            *buf_start = 0;
            *buf_end = 0;
        }
        // Keep reading until we have n bytes from buf_start
        while *buf_end - *buf_start < n {
            // Need more data
            let available = *buf_end - *buf_start;
            // Compact if there's consumed data
            if *buf_start > 0 && available > 0 {
                // Use safe copy instead of unsafe ptr::copy_nonoverlapping
                let data = buf[*buf_start..*buf_end].to_vec();
                buf[..available].copy_from_slice(&data);
                *buf_start = 0;
                *buf_end = available;
            } else if *buf_start > 0 {
                // No remaining data, just reset
                *buf_start = 0;
                *buf_end = 0;
            }
            // Grow buffer if at capacity (buf.len() bytes initialized, buf_end is within that)
            if *buf_end >= buf.len() {
                buf.resize(buf.len() * 2, 0);
            }
            // Read into uninitialized space at buf_end
            let read = stream.read(&mut buf[*buf_end..]).await?;
            if read == 0 {
                return Ok(false);
            }
            info!("[S7] <<< RECV {} bytes, head=[{}]", read, Self::hex_head(&buf[*buf_end..*buf_end + read], 10));
            *buf_end += read;
        }
        Ok(true)
    }

    /// Consume exactly `n` bytes from the front of the consumed region.
    /// Returns the consumed slice and advances buf_start.
    fn consume_buf<'a>(buf: &'a [u8], buf_start: &mut usize, buf_end: &mut usize, n: usize) -> Option<&'a [u8]> {
        if *buf_start + n > *buf_end {
            return None;
        }
        let consumed = &buf[*buf_start..*buf_start + n];
        *buf_start += n;
        Some(consumed)
    }

    /// Format first N bytes as hex string for logging
    fn hex_head(data: &[u8], n: usize) -> String {
        data.iter().take(n).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    }

    /// Send raw bytes through the stream, optionally wrapping in TPKT.
    async fn send_packet(
        stream: &mut tokio::net::TcpStream,
        payload: &[u8],
        use_tpkt: bool,
    ) -> Result<(), std::io::Error> {
        let mut full_buf: Vec<u8> = Vec::new();
        if use_tpkt {
            let total_len = payload.len() + 4;
            let tpkt: [u8; 4] = [
                0x03, 0x00,
                (total_len >> 8) as u8,
                (total_len & 0xFF) as u8,
            ];
            full_buf.extend_from_slice(&tpkt);
        }
        full_buf.extend_from_slice(payload);
        info!("[S7] >>> SEND {} bytes, head=[{}]", full_buf.len(), Self::hex_head(&full_buf, 10));
        stream.write_all(&full_buf).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Update a connection entry; if insert is Some, insert new entry
    fn update_connection(&self, conn_id: usize, framing: &str, state: &str, increment: u64, insert: Option<ClientConnection>) {
        let mut conns = self.connections.write().unwrap();
        if let Some(entry) = insert {
            conns.push(entry);
            return;
        }
        if let Some(c) = conns.iter_mut().find(|c| c.id == conn_id) {
            c.last_activity = chrono::Utc::now().to_rfc3339();
            c.requests_count += increment;
            if !framing.is_empty() { c.framing = framing.to_string(); }
            if !state.is_empty() { c.state = state.to_string(); }
        }
    }
    
    /// Remove a connection entry
    fn remove_connection(&self, conn_id: usize) {
        let mut conns = self.connections.write().unwrap();
        conns.retain(|c| c.id != conn_id);
    }

    pub async fn handle_connection(&self, mut stream: tokio::net::TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = stream.peer_addr()?;
        info!("[S7] +++ CONNECT from {}", addr);
        add_log_entry(&self.log_buffer, "INFO", &format!("S7 client connected from {}", addr));
        
        // Register connection
        let conn_id = CONNECTION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = chrono::Utc::now().to_rfc3339();
        let conn = ClientConnection {
            id: conn_id,
            remote_addr: addr.to_string(),
            connected_at: now.clone(),
            last_activity: now.clone(),
            requests_count: 0,
            framing: "detecting".to_string(),
            state: "connecting".to_string(),
        };
        self.update_connection(conn_id, "", "", 0, Some(conn));
        
        // Run the actual handler, then cleanup
        let result = self.handle_connection_inner(&mut stream, addr, conn_id).await;
        
        // Unregister connection
        self.remove_connection(conn_id);
        info!("[S7] --- DISCONNECT from {} (id={})", addr, conn_id);
        add_log_entry(&self.log_buffer, "INFO", &format!("S7 client disconnected from {} (id={})", addr, conn_id));
        
        result
    }
    
    /// Inner connection handler
    async fn handle_connection_inner(
        &self,
        mut stream: &mut tokio::net::TcpStream,
        addr: std::net::SocketAddr,
        conn_id: usize,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        // Disable Nagle to ensure immediate response
        let _ = stream.set_nodelay(true);
        
        // Pre-allocated zeroed buffer; always read at buf[buf_end] and compact when needed
        let mut buf = vec![0u8; 8192];
        let mut buf_start: usize = 0;
        let mut buf_end: usize = 0;
        
        // --- Read first byte to detect framing ---
        // Read at least 1 byte to decide: TPKT (0x03) vs raw COTP (0x11, 0x0D, 0x02, 0xF0, etc.)
        if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 1).await? {
            info!("[S7] --- CLOSE (EOF) from {}", addr);
            return Ok(());
        }
        
        // Detect framing: TPKT vs raw COTP
        let use_tpkt = buf[buf_start] == 0x03;
        debug!("[S7] Framing detected: {}", if use_tpkt { "TPKT (RFC 1006)" } else { "Raw COTP" });
        self.update_connection(conn_id, if use_tpkt { "TPKT" } else { "Raw COTP" }, "handshake", 0, None);
        
        // For TPKT, read 3 more bytes for full header (already have 1)
        if use_tpkt {
            // buf[buf_start] = 0x03 already read; need bytes buf_start+1, buf_start+2, buf_start+3
            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 4).await? {
                return Ok(());
            }
            let tpkt_len = ((buf[buf_start + 2] as usize) << 8) | (buf[buf_start + 3] as usize);
            // Consume 4-byte TPKT header via consume_buf
            let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, 4);
            
            // Read full COTP CR payload
            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, tpkt_len - 4).await? {
                return Ok(());
            }
            let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, tpkt_len - 4);
        } else {
            // Raw COTP: byte 0 = PDU type, byte 1 = LI (Length Indicator, total COTP length including type+LI)
            // COTP CR: 11 E0 ... → LI=0x11=17, type=0xE0
            // COTP DT: 02 F0 ... → LI=0x07=7, type=0x02
            // Read at least 2 bytes to get both type and LI
            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 2).await? {
                return Ok(());
            }
            let li = buf[buf_start + 1] as usize;
            // Read remaining COTP CR bytes
            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, li.saturating_sub(2)).await? {
                return Ok(());
            }
            let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, li);
        }
        
        // --- Build COTP CC response (match client framing) ---
        // COTP CC: 0x0D | LI(2) | dst-ref(2) | src-ref(2) | class(1) | TPDU-size(3) | calling-TSAP(4) | called-TSAP(4)
        // Total payload: 1+2+2+2+1+3+4+4 = 19 bytes → LI field = 0x13 (19)
        let cotp_cc: [u8; 19] = [
            0x0D,                   // CC type
            0x00, 0x13,             // LI = 19 (includes LI itself + 18 more bytes)
            0x00, 0x00,             // Dest ref (echo client's dst ref)
            0x00, 0x01,             // Src ref
            0x00,                   // Class
            0xC0, 0x01, 0x0A,      // TPDU size: 1024
            0xC1, 0x02, 0x00, 0x01, // Calling TSAP (echo)
            0xC2, 0x02, 0x00, 0x00, // Called TSAP (echo)
        ];
        
        // Send CC
        Self::send_packet(&mut stream, &cotp_cc, use_tpkt).await?;
        debug!("[S7] Sent COTP CC (TPKT={})", use_tpkt);
        
        // Note: Do NOT send COTP DT here — COTP DT is only used to carry S7 PDUs.
        // The client will send S7 Setup Communication next, and we respond in the main loop.
        
        // --- Main request loop ---
        info!("[S7] Entering main loop, use_tpkt={}, buf_start={}, buf_end={}", use_tpkt, buf_start, buf_end);
        loop {
            if use_tpkt {
                // TPKT mode: always read TPKT header first
                if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 4).await? {
                    info!("[S7] --- CLOSE (EOF) from {}", addr);
                    return Ok(());
                }
                if buf[buf_start] != 0x03 {
                    warn!("[S7] Expected TPKT 0x03, got {:#x}, closing", buf[buf_start]);
                    return Ok(());
                }
                let tpkt_len = ((buf[buf_start + 2] as usize) << 8) | (buf[buf_start + 3] as usize);
                info!("[S7] TPKT header: ver={:#x}, len={}, buf_start={}, buf_end={}", buf[buf_start], tpkt_len, buf_start, buf_end);
                // Consume the 4-byte TPKT header via consume_buf for consistency
                let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, 4);
                
                if tpkt_len < 4 {
                    warn!("[S7] Invalid TPKT length {}, closing", tpkt_len);
                    return Ok(());
                }
                let payload_len = tpkt_len - 4;
                if payload_len > 0 {
                    if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, payload_len).await? {
                        return Ok(());
                    }
                }
                let payload = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, payload_len)
                    .unwrap_or(&[][..]);
                self.process_payload(&payload, &addr, &mut stream, use_tpkt, conn_id).await?;
            } else {
                // Raw mode: peek first byte to detect frame type
                let avail = buf_end - buf_start;
                if avail > 0 {
                    info!("[S7] Raw mode, {} leftover bytes, first={:#x}", avail, buf[buf_start]);
                }
                if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 1).await? {
                    info!("[S7] --- CLOSE (EOF) from {}", addr);
                    return Ok(());
                }
                let first = buf[buf_start];
                info!("[S7] Raw first byte: {:#x} (buf_start={}, buf_end={})", first, buf_start, buf_end);
                
                if first == PROTOCOL_ID {
                    // S7 packet: process from buffer without blocking
                    // S7 PDU: bytes 0-9 header, param_len at bytes 5-6, data_len at bytes 7-8
                    let avail = buf_end - buf_start;
                    if avail < 10 {
                        if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 10 - avail).await? {
                            return Ok(());
                        }
                    }
                    // S7 header: [0:proto][1:type][2-3:rsv][4-5:ref][6-7:param_len][8-9:data_len]
                    let param_len = ((buf[buf_start + 6] as usize) << 8) | (buf[buf_start + 7] as usize);
                    let data_len  = ((buf[buf_start + 8] as usize) << 8) | (buf[buf_start + 9] as usize);
                    let s7_total = 10 + param_len + data_len;
                    
                    if buf_end - buf_start < s7_total {
                        let need = s7_total - (buf_end - buf_start);
                        if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, need).await? {
                            return Ok(());
                        }
                    }
                    let payload = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, s7_total)
                        .unwrap_or(&[][..]);
                    info!("[S7] S7 packet processed ({} bytes, param={}, data={})", s7_total, param_len, data_len);
                    self.process_payload(payload, &addr, &mut stream, use_tpkt, conn_id).await?;
                } else {
                    // COTP: check PDU type to determine length
                    if first == 0x02 {
                        // COTP DT: fixed 7 bytes: [type=02 | TPDU=0xF0 | EOT | dst-ref(2) | src-ref(2)]
                        // No LI field; always exactly 7 bytes
                        if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 6).await? {
                            return Ok(());
                        }
                        let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, 7);
                        debug!("[S7] COTP DT consumed (7 bytes)");
                        
                        // COTP DT may be followed by S7 data in same TCP segment
                        // Check if there's an S7 packet already in buffer
                        let avail = buf_end - buf_start;
                        let dump_len = (buf_end - buf_start).min(40);
                        info!("[S7] After COTP DT: bs={} be={} avail={} buf[{}..{}]={:02X?}",
                              buf_start, buf_end, avail, buf_start, buf_start + dump_len, &buf[buf_start..buf_start + dump_len]);
                        if avail >= 10 && buf[buf_start] == PROTOCOL_ID {
                            // Peek: read S7 header to get total length
                            let param_len = ((buf[buf_start + 6] as usize) << 8) | (buf[buf_start + 7] as usize);
                            let data_len  = ((buf[buf_start + 8] as usize) << 8) | (buf[buf_start + 9] as usize);
                            let s7_total = 10 + param_len + data_len;
                            info!("[S7] COTP-DT S7 detect: param_len={} data_len={} s7_total={} avail={}",
                                  param_len, data_len, s7_total, avail);
                            if avail >= s7_total {
                                info!("[S7] COTP-DT S7 processing {} bytes...", s7_total);
                                let payload = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, s7_total)
                                    .unwrap_or(&[][..]);
                                self.process_payload(payload, &addr, &mut stream, use_tpkt, conn_id).await?;
                                info!("[S7] COTP-DT S7 done, bs={} be={}", buf_start, buf_end);
                            } else {
                                info!("[S7] COTP-DT S7 need more: {} < {}", avail, s7_total);
                            }
                        }
                    } else if first == 0x0D || first == 0xE0 || first == 0x08 || first == 0x07 || first == 0xF0 {
                        // Known COTP PDU types: CR(0xE0), CC(0x0D), DR(0x80), DC(0xF0), etc.
                        // COTP: PDU type at buf_start, LI (Length Indicator) at buf_start+1
                        // The client sends: [type=0xE0 | LI=0x11 | ...] so LI is at buf_start+1
                        let avail = buf_end - buf_start;
                        if avail < 2 {
                            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 2 - avail).await? {
                                return Ok(());
                            }
                        }
                        let li = buf[buf_start + 1] as usize;
                        if li < 2 || li > 255 {
                            warn!("[S7] Invalid COTP LI={} at buf_start={}, skipping", li, buf_start);
                            let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, 1);
                            continue;
                        }
                        let cotp_total = li;
                        // Read remaining COTP bytes
                        let avail = buf_end - buf_start;
                        let need = cotp_total.saturating_sub(avail);
                        if need > 0 {
                            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, need).await? {
                                return Ok(());
                            }
                        }
                        
                        // Consume COTP PDU and peek what follows
                        let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, cotp_total);
                        debug!("[S7] COTP consumed {} bytes (type={:#x})", cotp_total, first);
                        
                        // Peek remaining bytes for S7 detection
                        let avail = buf_end - buf_start;
                        if avail >= 1 && buf[buf_start] == PROTOCOL_ID {
                            // S7 packet follows - read full S7 PDU
                            let need_s7 = 10usize.saturating_sub(avail);
                            if need_s7 > 0 {
                                if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, need_s7).await? {
                                    return Ok(());
                                }
                            }
                            let param_len = ((buf[buf_start + 6] as usize) << 8) | (buf[buf_start + 7] as usize);
                            let data_len  = ((buf[buf_start + 8] as usize) << 8) | (buf[buf_start + 9] as usize);
                            let s7_total = 10 + param_len + data_len;
                            
                            let need_s7_data = s7_total.saturating_sub(buf_end - buf_start);
                            if need_s7_data > 0 {
                                if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, need_s7_data).await? {
                                    return Ok(());
                                }
                            }
                            let payload = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, s7_total)
                                .unwrap_or(&[][..]);
                            self.process_payload(payload, &addr, &mut stream, use_tpkt, conn_id).await?;
                        }
                    } else {
                        // Unknown byte - skip it (could be leftover data from previous packet)
                        warn!("[S7] Unknown byte {:#x} at buf_start={}, skipping", first, buf_start);
                        let _ = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, 1);
                    }
                }
                continue;
            }
        }
    }

    
    /// Process a TPKT payload (may contain COTP DT and/or S7 packets)
    async fn process_payload(&self, payload: &[u8], addr: &std::net::SocketAddr, stream: &mut tokio::net::TcpStream, use_tpkt: bool, conn_id: usize) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("[S7] process_payload: {} bytes, head=[{}]", payload.len(), Self::hex_head(payload, 10));
        let mut offset = 0;
        
        while offset < payload.len() {
            let remaining = &payload[offset..];
            
            if remaining.is_empty() {
                break;
            }
            
            // Check if it's an S7 packet (starts with 0x32)
            if remaining[0] == PROTOCOL_ID {
                // S7 header: 10 bytes
                if remaining.len() < 10 {
                    break;
                }
                
                let pdu_type = remaining[1];
                // S7 PDU header: [0:proto][1:type][2-3:reserved][4-5:ref][6-7:param_len][8-9:data_len]
                let pdu_ref = ((remaining[4] as u16) << 8) | (remaining[5] as u16);
                let param_len = ((remaining[6] as u16) << 8) | (remaining[7] as u16);
                let data_len = ((remaining[8] as u16) << 8) | (remaining[9] as u16);
                let total_len = 10 + param_len as usize + data_len as usize;
                
                if remaining.len() < total_len {
                    break;
                }
                
                let mut full_packet = remaining[..total_len].to_vec();
                offset += total_len;
                
                // Handle S7 request
                let response = self.handle_request(pdu_type, pdu_ref, &full_packet[10..], conn_id).await;
                
                // Send TPKT + COTP DT + S7 response
                if let Some(resp) = response {
                    if use_tpkt {
                        // TPKT payload = COTP DT header (3 bytes) + S7 PDU
                        let cotp_dt_header: [u8; 3] = [0x02, 0xF0, 0x80];
                        let payload_len = cotp_dt_header.len() + resp.len();
                        let tpkt_resp: Vec<u8> = vec![
                            0x03, 0x00,
                            ((payload_len + 4) >> 8) as u8, ((payload_len + 4) & 0xFF) as u8,
                        ];
                        let mut send_buf = Vec::with_capacity(4 + 3 + resp.len());
                        send_buf.extend_from_slice(&tpkt_resp);
                        send_buf.extend_from_slice(&cotp_dt_header);
                        send_buf.extend_from_slice(&resp);
                        info!("[S7] >>> SEND {} bytes (TPKT+COTP+S7), head=[{}]", send_buf.len(), Self::hex_head(&send_buf, 10));
                        stream.write_all(&send_buf).await?;
                    } else {
                        info!("[S7] >>> SEND {} bytes (S7), head=[{}]", resp.len(), Self::hex_head(&resp, 10));
                        stream.write_all(&resp).await?;
                    }
                    stream.flush().await?;
                    debug!("[S7] Sent response: {} bytes (TPKT={})", resp.len(), use_tpkt);
                }
            } else if remaining[0] == 0x02 {
                // COTP DT: [02 | F0 | 80] — 3 bytes, no LI field
                // Just skip the 3-byte COTP DT header, then continue processing S7 data
                let dt_header_len = if remaining.len() >= 3 && remaining[2] == 0x80 {
                    3 // [02, F0, 80]
                } else if remaining.len() >= 2 {
                    2 // [02, F0] — minimal DT header
                } else {
                    1 // just the type byte
                };
                debug!("[S7] Skipping COTP DT header: {} bytes", dt_header_len);
                offset += dt_header_len;
            } else {
                // Other COTP packet (CR, CC, DR, etc.): [type(1) | LI(1) | rest(LI-2 bytes)]
                let cotp_total = remaining.len().min(if remaining.len() >= 2 { remaining[1] as usize } else { 1 });
                debug!("[S7] Skipping COTP PDU type={:#x}: {} bytes", remaining[0], cotp_total);
                offset += cotp_total;
            }
        }
        
        Ok(())
    }
    
    /// Handle S7 request
    pub(crate) async fn handle_request(&self, pdu_type: u8, pdu_ref: u16, param_data: &[u8], conn_id: usize) -> Option<Vec<u8>> {
        match pdu_type {
            0x01 => { // Job
                if param_data.len() < 1 {
                    return None;
                }
                
                let function_code = param_data[0];
                
                match function_code {
                    0xF0 => { // Setup Communication
                        debug!("Setup Communication");
                        self.update_connection(conn_id, "", "connected", 0, None);
                        Some(self.build_setup_response(pdu_ref))
                    }
                    0x04 => { // Read
                        debug!("Read request");
                        self.update_connection(conn_id, "", "", 1, None);
                        self.handle_read_request(pdu_ref, param_data).await
                    }
                    0x05 => { // Write
                        debug!("Write request");
                        self.update_connection(conn_id, "", "", 1, None);
                        self.handle_write_request(pdu_ref, param_data).await
                    }
                    _ => {
                        warn!("Unknown function: {:#x}", function_code);
                        None
                    }
                }
            }
            0x07 => { // User data
                Some(self.build_user_data_response(pdu_ref))
            }
            _ => None,
        }
    }
    
    /// Build setup communication response
    pub(crate) fn build_setup_response(&self, pdu_ref: u16) -> Vec<u8> {
        vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            0x00, 0x08, // Param len
            0x00, 0x00, // Data len
            FunctionCode::SetupCommunication as u8, 0x00,
            0x03, 0xE8, // Max AmQ
            0x03, 0xE8, // Max AmQ
            0x01, 0xE0, // PDU size (480)
        ]
    }
    
    /// Handle read request (S7 standard response format)
    #[allow(dead_code)]
    /// 
    /// Request param_data: [function_code(0x04), item_count, item_spec...]
    /// 
    /// Response format (S7 standard):
    ///   Param section: [function_code(0x04), item_count] (2 bytes)
    ///   Data section (per item): [return_code, transport_size, data_len_hi, data_len_lo, actual_data...]
    pub(crate) async fn handle_read_request(&self, pdu_ref: u16, param_data: &[u8]) -> Option<Vec<u8>> {
        if param_data.len() < 2 {
            return None;
        }
        
        let item_count = param_data[1] as usize;
        let memory = self.memory.read().ok()?;
        
        // Data section: per-item [return_code, transport_size, len_hi, len_lo, data...]
        let mut data_results: Vec<u8> = Vec::new();
        let mut read_items = Vec::new();
        
        let mut offset = 2;  // After function code + item count
        
        for _ in 0..item_count {
            if offset + 2 > param_data.len() { break; }
            
            let spec_type = param_data[offset];
            let spec_len = param_data[offset + 1] as usize;
            
            // S7 Read Var item spec: 0x12 | spec_len(0x0A=10) | syntax_id(0x10) |
            //   transport_size(1) | num_elements(2) | db_number(2) | area_code(1) | offset(3)
            // Total spec data after 0x12+spec_len = 10 bytes
            if spec_type != 0x12 || spec_len < 10 || offset + 2 + spec_len > param_data.len() {
                offset += 2;
                // Push error item in data section
                data_results.push(0x0A); // Error: data not available
                data_results.push(0x00);
                data_results.push(0x00);
                data_results.push(0x00);
                continue;
            }
            
            let transport = param_data[offset + 2];
            let num_elements = ((param_data[offset + 4] as u16) << 8) | (param_data[offset + 5] as u16);
            let db_num = ((param_data[offset + 6] as u16) << 8) | (param_data[offset + 7] as u16);
            let area_code = param_data[offset + 8];
            // Offset is 3 bytes: high byte contains bit offset in bits 0-2, byte offset in bits 3-7;
            //   low 2 bytes are the main byte offset
            let offset_byte0 = param_data[offset + 9] as u32;
            let offset_byte1 = param_data[offset + 10] as u32;
            let offset_byte2 = param_data[offset + 11] as u32;
            let start = ((offset_byte0 & 0x00FF) << 16) | (offset_byte1 << 8) | offset_byte2;
            
            let area = MemoryArea::from_byte(area_code);
            
            if let Some(area) = area {
                match memory.read(area, db_num, start as usize, num_elements as usize) {
                    Some(data) => {
                        data_results.push(0xFF);                          // Success return code
                        data_results.push(TransportSize::Byte as u8);     // Transport size
                        data_results.push((data.len() >> 8) as u8);       // Data length high
                        data_results.push((data.len() & 0xFF) as u8);     // Data length low
                        data_results.extend(&data);
                        
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        read_items.push(format!("{:?}[{}] offset={} len={} data=[{}]",
                            area, db_num, start, num_elements, hex_str));
                    }
                    None => {
                        data_results.push(0x0A);  // Error: data not available
                        data_results.push(0x00);
                        data_results.push(0x00);
                        data_results.push(0x00);
                        read_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, num_elements));
                    }
                }
            } else {
                data_results.push(0x0A);  // Error: unknown area
                data_results.push(0x00);
                data_results.push(0x00);
                data_results.push(0x00);
            }
            
            let _ = transport;
            offset += 2 + spec_len;
        }
        
        // Param section: only function_code + item_count
        let param_len = 2u16;
        let data_len = data_results.len() as u16;
        
        let mut response = vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            (param_len >> 8) as u8, (param_len & 0xFF) as u8,
            (data_len >> 8) as u8, (data_len & 0xFF) as u8,
            FunctionCode::Read as u8, item_count as u8,
        ];
        response.extend(data_results);
        
        if !read_items.is_empty() {
            info!("[READ] items={} {}", read_items.len(), read_items.join(" | "));
            add_log_entry(&self.log_buffer, "INFO", &format!("S7 Read: {}", read_items.join(" | ")));
        }
        
        Some(response)
    }
    
    /// Handle write request (S7 standard response format)
    #[allow(dead_code)]
    /// 
    /// Response format (S7 standard):
    ///   Param section: [function_code(0x05), item_count] (2 bytes)
    ///   Data section: [return_code(0xFF)] per item (1 byte each)
    pub(crate) async fn handle_write_request(&self, pdu_ref: u16, param_data: &[u8]) -> Option<Vec<u8>> {
        if param_data.len() < 2 {
            return None;
        }
        
        let item_count = param_data[1] as usize;
        let mut memory = self.memory.write().ok()?;
        
        let mut offset = 2;  // After function code + item count
        let mut success_count = 0;
        let mut write_items = Vec::new();
        // Data section: one return code byte per item
        let mut data_results: Vec<u8> = Vec::new();
        
        for _ in 0..item_count {
            if offset + 2 > param_data.len() { break; }
            
            let spec_type = param_data[offset];
            let spec_len = param_data[offset + 1] as usize;
            
            // S7 Write Var item spec: same format as Read
            if spec_type != 0x12 || spec_len < 10 || offset + 2 + spec_len > param_data.len() {
                offset += 2;
                data_results.push(0x0A); // Error
                continue;
            }
            
            let num_elements = ((param_data[offset + 4] as u16) << 8) | (param_data[offset + 5] as u16);
            let db_num = ((param_data[offset + 6] as u16) << 8) | (param_data[offset + 7] as u16);
            let area_code = param_data[offset + 8];
            // Offset is 3 bytes (same as Read)
            let off0 = param_data[offset + 9] as u32;
            let off1 = param_data[offset + 10] as u32;
            let off2 = param_data[offset + 11] as u32;
            let start = ((off0 & 0x00FF) << 16) | (off1 << 8) | off2;
            
            let area = MemoryArea::from_byte(area_code);
            
            let write_ok = if let Some(area) = area {
                let data_start = offset + 2 + spec_len;
                let data_end = data_start + num_elements as usize;
                if data_end <= param_data.len() {
                    let data = &param_data[data_start..data_end];
                    if memory.write(area, db_num, start as usize, data) {
                        success_count += 1;
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        write_items.push(format!("{:?}[{}] offset={} len={} data=[{}] OK",
                            area, db_num, start, num_elements, hex_str));
                        true
                    } else {
                        write_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, num_elements));
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            
            data_results.push(if write_ok { 0xFF } else { 0x0A });
            offset += 2 + spec_len;
        }
        
        // Param section: only function_code + item_count
        let param_len = 2u16;
        let data_len = data_results.len() as u16;
        
        let response = vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            (param_len >> 8) as u8, (param_len & 0xFF) as u8,
            (data_len >> 8) as u8, (data_len & 0xFF) as u8,
            FunctionCode::Write as u8, item_count as u8,
        ];
        
        let mut full_response = response;
        full_response.extend(data_results);
        
        if !write_items.is_empty() {
            info!("[WRITE] items={}/{} {}", success_count, item_count, write_items.join(" | "));
            add_log_entry(&self.log_buffer, "INFO", &format!("S7 Write: {}", write_items.join(" | ")));
        }
        
        Some(full_response)
    }
    
    /// Build user data response
    pub(crate) fn build_user_data_response(&self, pdu_ref: u16) -> Vec<u8> {
        vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            0x00, 0x04, // Param len
            0x00, 0x00, // Data len
            0x00, 0x00, 0x00, 0x00,
        ]
    }
    
    /// Start S7 server
    pub async fn start_s7_server(port: u16, memory: SharedMemory, plc_type: &str, rack: u8, slot: u8, connections: ConnectionList, log_buffer: LogBuffer) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!("0.0.0.0:{}", port);
        info!("S7 Server listening on {}", addr);
        
        let listener = TcpListener::bind(&addr).await?;
        
        loop {
            if let Ok((stream, addr)) = listener.accept().await {
                info!("[S7] Incoming connection from {}", addr);
                let simulator = PlcSimulator::new(plc_type, rack, slot, memory.clone(), connections.clone(), log_buffer.clone());
                tokio::spawn(async move {
                    if let Err(e) = simulator.handle_connection(stream).await {
                        error!("[S7] Handler error from {}: {}", addr, e);
                    }
                });
            }
        }
    }
}
