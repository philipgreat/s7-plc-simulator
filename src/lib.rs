//!
//! S7 PLC Simulator
//! 
//! A simulated S7 PLC with Web Admin API

pub mod memory;
pub mod api;

use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

pub use memory::{PlcMemory, SharedMemory, create_shared_memory, MemoryArea};

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
}

impl PlcSimulator {
    /// Create new PLC simulator
    pub fn new(plc_type: &str, rack: u8, slot: u8, memory: SharedMemory) -> Self {
        Self {
            memory,
            plc_type: plc_type.to_string(),
            rack,
            slot,
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
        // Keep reading until we have n bytes from buf_start
        while *buf_end - *buf_start < n {
            // Need more data
            let available = *buf_end - *buf_start;
            // Compact if there's consumed data
            if *buf_start > 0 && available > 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        buf.as_ptr().add(*buf_start),
                        buf.as_mut_ptr(),
                        available,
                    );
                }
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

    /// Send raw bytes through the stream, optionally wrapping in TPKT.
    async fn send_packet(
        stream: &mut tokio::net::TcpStream,
        payload: &[u8],
        use_tpkt: bool,
    ) -> Result<(), std::io::Error> {
        if use_tpkt {
            let total_len = payload.len() + 4;
            let tpkt: [u8; 4] = [
                0x03, 0x00,
                (total_len >> 8) as u8,
                (total_len & 0xFF) as u8,
            ];
            stream.write_all(&tpkt).await?;
        }
        stream.write_all(payload).await?;
        stream.flush().await?;
        Ok(())
    }

    pub async fn handle_connection(&self, mut stream: tokio::net::TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = stream.peer_addr()?;
        info!("[S7] +++ CONNECT from {}", addr);
        
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
        
        // For TPKT, read 3 more bytes for full header (already have 1)
        if use_tpkt {
            // buf[buf_start] = 0x03 already read; need bytes buf_start+1, buf_start+2, buf_start+3
            if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, 4).await? {
                return Ok(());
            }
            let tpkt_len = ((buf[buf_start + 2] as usize) << 8) | (buf[buf_start + 3] as usize);
            buf_start += 4;
            
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
        
        // Send COTP DT (connection active) - same framing as CC
        let cotp_dt: [u8; 7] = [0x02, 0xF0, 0x80, 0x00, 0x00, 0x00, 0x00];
        Self::send_packet(&mut stream, &cotp_dt, use_tpkt).await?;
        debug!("[S7] Sent COTP DT (TPKT={})", use_tpkt);
        
        // --- Main request loop ---
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
                buf_start += 4;
                
                if !Self::read_exact_buf(&mut stream, &mut buf, &mut buf_start, &mut buf_end, tpkt_len - 4).await? {
                    return Ok(());
                }
                let payload = Self::consume_buf(&buf, &mut buf_start, &mut buf_end, tpkt_len - 4)
                    .unwrap_or(&[][..]);
                self.process_payload(&payload, &addr, &mut stream, use_tpkt).await?;
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
                    self.process_payload(payload, &addr, &mut stream, use_tpkt).await?;
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
                                self.process_payload(payload, &addr, &mut stream, use_tpkt).await?;
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
                            self.process_payload(payload, &addr, &mut stream, use_tpkt).await?;
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
    async fn process_payload(&self, payload: &[u8], addr: &std::net::SocketAddr, stream: &mut tokio::net::TcpStream, use_tpkt: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
                let response = self.handle_request(pdu_type, pdu_ref, &full_packet[10..]).await;
                
                // Send TPKT-wrapped response
                if let Some(resp) = response {
                    if use_tpkt {
                        let tpkt_resp: Vec<u8> = vec![
                            0x03, 0x00,
                            ((resp.len() + 4) >> 8) as u8, ((resp.len() + 4) & 0xFF) as u8,
                        ];
                        stream.write_all(&tpkt_resp).await?;
                    }
                    stream.write_all(&resp).await?;
                    stream.flush().await?;
                    debug!("[S7] Sent response: {} bytes (TPKT={})", resp.len(), use_tpkt);
                }
            } else {
                // COTP packet (DT or other)
                // Raw COTP: [type(1) | LI(1) | rest(LI-2 bytes)]
                // COTP DT: 02 06 F0 80 00 00 00 → type=02, LI=06 (6 total bytes)
                let cotp_total = remaining.len().min(if remaining.len() >= 2 { remaining[1] as usize } else { 1 });
                debug!("[S7] Skipping COTP PDU: {} bytes", cotp_total);
                offset += cotp_total;
            }
        }
        
        Ok(())
    }
    
    /// Handle S7 request
    async fn handle_request(&self, pdu_type: u8, pdu_ref: u16, param_data: &[u8]) -> Option<Vec<u8>> {
        match pdu_type {
            0x01 => { // Job
                if param_data.len() < 1 {
                    return None;
                }
                
                let function_code = param_data[0];
                
                match function_code {
                    0xF0 => { // Setup Communication
                        debug!("Setup Communication");
                        Some(self.build_setup_response(pdu_ref))
                    }
                    0x04 => { // Read
                        debug!("Read request");
                        self.handle_read_request(pdu_ref, param_data).await
                    }
                    0x05 => { // Write
                        debug!("Write request");
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
    fn build_setup_response(&self, pdu_ref: u16) -> Vec<u8> {
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
    
    /// Handle read request
    /// param_data: full S7 params = [function_code, item_count, item_spec...]
    async fn handle_read_request(&self, pdu_ref: u16, param_data: &[u8]) -> Option<Vec<u8>> {
        // S7 Read Request format:
        // Byte 0: Function code (0x04)
        // Byte 1: Item count
        // For each item:
        //   Byte 0: Specification type (0x12 = variable)
        //   Byte 1: Length of spec data that follows (typically 0x0A = 10)
        //   Bytes 2-3: Transport size (0x10) + reserved
        //   Bytes 4-5: Number of elements (big-endian)
        //   Bytes 6-7: DB number (big-endian)
        //   Byte 8: Area code
        //   Bytes 9-10-11: Byte offset (3 bytes)
        // Total per item: 2 (header) + var_len bytes of spec

        if param_data.len() < 2 {
            return None;
        }
        
        let item_count = param_data[1] as usize;
        let memory = self.memory.read().ok()?;
        
        // Param section of response: function(1) + count(1) + per-item result(4 bytes each)
        // Data section of response: per-item actual bytes
        let mut param_results = Vec::new();  // goes into param section
        let mut data_results: Vec<u8> = Vec::new();    // goes into data section
        let mut read_items = Vec::new();
        
        let mut offset = 2;  // After function code + item count
        
        for _ in 0..item_count {
            if offset + 2 > param_data.len() { break; }
            
            let spec_type = param_data[offset];
            let spec_len = param_data[offset + 1] as usize;
            
            // Need at least 2 (header) + spec_len bytes
            if spec_type != 0x12 || spec_len < 8 || offset + 2 + spec_len > param_data.len() {
                offset += 2;
                // Push error result
                param_results.push(0x0A);
                param_results.push(0x00);
                param_results.push(0x00);
                param_results.push(0x00);
                continue;
            }
            
            let transport = param_data[offset + 2];
            let num_elements = ((param_data[offset + 4] as u16) << 8) | (param_data[offset + 5] as u16);
            let db_num = ((param_data[offset + 6] as u16) << 8) | (param_data[offset + 7] as u16);
            let area_code = param_data[offset + 8];
            let start = ((param_data[offset + 9] as u16) << 8) | (param_data[offset + 10] as u16);
            // For 3-byte offset: start = (hi << 16) | (mid << 8) | lo
            // But for S7-300, 2 bytes is enough
            
            let area = MemoryArea::from_byte(area_code);
            
            let result = if let Some(area) = area {
                match memory.read(area, db_num, start as usize, num_elements as usize) {
                    Some(data) => {
                        param_results.push(0xFF);                          // Success
                        param_results.push(TransportSize::Byte as u8);     // Transport size
                        param_results.push((data.len() >> 8) as u8);       // Data length high
                        param_results.push((data.len() & 0xFF) as u8);     // Data length low
                        data_results.extend(&data);
                        
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        read_items.push(format!("{:?}[{}] offset={} len={} data=[{}]",
                            area, db_num, start, num_elements, hex_str));
                        true
                    }
                    None => {
                        param_results.push(0x0A);  // Error: data not available
                        param_results.push(0x00);
                        param_results.push(0x00);
                        param_results.push(0x00);
                        read_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, num_elements));
                        false
                    }
                }
            } else {
                param_results.push(0x0A);  // Error: unknown area
                param_results.push(0x00);
                param_results.push(0x00);
                param_results.push(0x00);
                false
            };
            
            let _ = (transport, result);
            offset += 2 + spec_len;
        }
        
        let param_len = 2 + param_results.len();  // function + count + results
        let data_len = data_results.len();
        
        let mut response = vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            (param_len >> 8) as u8, (param_len & 0xFF) as u8,
            (data_len >> 8) as u8, (data_len & 0xFF) as u8,
            FunctionCode::Read as u8, item_count as u8,
        ];
        response.extend(param_results);
        response.extend(data_results);
        
        if !read_items.is_empty() {
            info!("[READ] items={} {}", read_items.len(), read_items.join(" | "));
        }
        
        Some(response)
    }
    
    /// Handle write request
    /// param_data: full S7 params = [function_code, item_count, item_spec...]
    async fn handle_write_request(&self, pdu_ref: u16, param_data: &[u8]) -> Option<Vec<u8>> {
        if param_data.len() < 2 {
            return None;
        }
        
        let item_count = param_data[1] as usize;
        let mut memory = self.memory.write().ok()?;
        
        let mut offset = 2;  // After function code + item count
        let mut success_count = 0;
        let mut write_items = Vec::new();
        
        for _ in 0..item_count {
            if offset + 2 > param_data.len() { break; }
            
            let spec_type = param_data[offset];
            let spec_len = param_data[offset + 1] as usize;
            
            if spec_type != 0x12 || spec_len < 8 || offset + 2 + spec_len > param_data.len() {
                offset += 2;
                continue;
            }
            
            let num_elements = ((param_data[offset + 4] as u16) << 8) | (param_data[offset + 5] as u16);
            let db_num = ((param_data[offset + 6] as u16) << 8) | (param_data[offset + 7] as u16);
            let area_code = param_data[offset + 8];
            let start = ((param_data[offset + 9] as u16) << 8) | (param_data[offset + 10] as u16);
            
            let area = MemoryArea::from_byte(area_code);
            
            if let Some(area) = area {
                let data_start = offset + 2 + spec_len;
                let data_end = data_start + num_elements as usize;
                if data_end <= param_data.len() {
                    let data = &param_data[data_start..data_end];
                    if memory.write(area, db_num, start as usize, data) {
                        success_count += 1;
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        write_items.push(format!("{:?}[{}] offset={} len={} data=[{}] OK",
                            area, db_num, start, num_elements, hex_str));
                    } else {
                        write_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, num_elements));
                    }
                }
            }
            
            offset += 2 + spec_len;
        }
        
        let response = vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            0x00, 0x02, // Param len
            0x00, item_count as u8, // Data len
            FunctionCode::Write as u8, item_count as u8,
        ];
        
        if !write_items.is_empty() {
            info!("[WRITE] items={}/{} {}", success_count, item_count, write_items.join(" | "));
        }
        
        Some(response)
    }
    
    /// Build user data response
    fn build_user_data_response(&self, pdu_ref: u16) -> Vec<u8> {
        vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            0x00, 0x04, // Param len
            0x00, 0x00, // Data len
            0x00, 0x00, 0x00, 0x00,
        ]
    }
    
    /// Start S7 server
    pub async fn start_s7_server(port: u16, memory: SharedMemory, plc_type: &str, rack: u8, slot: u8) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!("0.0.0.0:{}", port);
        info!("S7 Server listening on {}", addr);
        
        let listener = TcpListener::bind(&addr).await?;
        
        loop {
            if let Ok((stream, addr)) = listener.accept().await {
                info!("[S7] Incoming connection from {}", addr);
                let simulator = PlcSimulator::new(plc_type, rack, slot, memory.clone());
                tokio::spawn(async move {
                    if let Err(e) = simulator.handle_connection(stream).await {
                        error!("[S7] Handler error from {}: {}", addr, e);
                    }
                });
            }
        }
    }
}
