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
    pub async fn handle_connection(&self, mut stream: tokio::net::TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = stream.peer_addr()?;
        info!("[S7] +++ CONNECT from {}", addr);
        
        // Buffered reader to handle multiple TPKT packets in one TCP stream
        use tokio::io::ReadBuf;
        let mut buf = vec![0u8; 8192];
        let mut buf_start = 0;
        let mut buf_end = 0;
        
        // Read TPKT header (4 bytes): version, reserved, length(2 bytes big-endian)
        loop {
            if buf_start + 4 > buf_end {
                // Need more data
                let remaining = buf_end - buf_start;
                buf.copy_within(buf_start..buf_end, 0);
                buf_start = 0;
                buf_end = remaining;
                let n = stream.read(&mut buf[buf_end..]).await?;
                if n == 0 {
                    info!("[S7] --- CLOSE (EOF) from {}", addr);
                    return Ok(());
                }
                buf_end += n;
                continue;
            }
            break;
        }
        
        // Accept TPKT v3 (0x03)
        if buf[buf_start] != 0x03 {
            warn!("[S7] Unknown TPKT version: {:#x}, closing connection", buf[buf_start]);
            return Ok(());
        }
        
        // TPKT length includes all bytes (header + payload)
        let tpkt_len = ((buf[buf_start + 2] as usize) << 8) | (buf[buf_start + 3] as usize);
        let payload_len = tpkt_len - 4;  // Subtract TPKT header
        buf_start += 4;
        
        // Read COTP payload
        loop {
            if buf_start + payload_len > buf_end {
                let remaining = buf_end - buf_start;
                buf.copy_within(buf_start..buf_end, 0);
                buf_start = 0;
                buf_end = remaining;
                let n = stream.read(&mut buf[buf_end..]).await?;
                if n == 0 {
                    info!("[S7] --- CLOSE (EOF) from {}", addr);
                    return Ok(());
                }
                buf_end += n;
                continue;
            }
            break;
        }
        
        let cotp_payload = &buf[buf_start..buf_start + payload_len];
        buf_start += payload_len;
        debug!("[S7] COTP CR: {} bytes", payload_len);
        
        // Send COTP CC wrapped in TPKT
        // COTP CC structure: 1-byte code + 1-byte length + 2-byte dst-ref + 2-byte src-ref + 1-byte class + optional params
        // Optional params: TPDU-size(3) + calling-TSAP(4) + called-TSAP(4) = 11 bytes
        // Total: 7 + 11 = 18 bytes COTP payload → COTP length field = 18
        let cotp_cc = vec![
            0x0D,                   // CC code
            0x00, 0x12,             // Length: 18 (COTP payload after this byte)
            0x00, 0x00,             // Dest ref
            0x00, 0x01,             // Src ref
            0x00,                   // Class
            0xC0, 0x01, 0x0A,      // TPDU size: 1024
            0xC1, 0x02, 0x00, 0x01, // Calling TSAP
            0xC2, 0x02, 0x00, 0x00, // Called TSAP
        ];
        let tpkt_cc_len = cotp_cc.len() + 4;  // 22 + 4 = 26
        let tpkt_cc = vec![
            0x03, 0x00,
            (tpkt_cc_len >> 8) as u8, (tpkt_cc_len & 0xFF) as u8,
        ];
        stream.write_all(&tpkt_cc).await?;
        stream.write_all(&cotp_cc).await?;
        stream.flush().await?;
        debug!("[S7] Sent COTP CC (TPKT len={})", tpkt_cc_len);
        
        // Send COTP DT (activate) as separate TCP packet
        // COTP DT: 1-byte DT code + 3 bytes TPDU + 3 bytes padding = 7 bytes COTP payload
        let cotp_dt: [u8; 7] = [0x02, 0xF0, 0x80, 0x00, 0x00, 0x00, 0x00];
        let tpkt_dt_len = cotp_dt.len() + 4;  // 7 + 4 = 11
        let tpkt_dt: [u8; 4] = [
            0x03, 0x00,
            (tpkt_dt_len >> 8) as u8, (tpkt_dt_len & 0xFF) as u8,
        ];
        stream.write_all(&tpkt_dt).await?;
        stream.write_all(&cotp_dt).await?;
        stream.flush().await?;
        debug!("[S7] Sent COTP DT (TPKT len={})", tpkt_dt_len);
        
        // Main request loop - read TPKT header first, then S7 packet
        loop {
            // Read TPKT header (4 bytes)
            loop {
                if buf_start + 4 > buf_end {
                    let remaining = buf_end - buf_start;
                    buf.copy_within(buf_start..buf_end, 0);
                    buf_start = 0;
                    buf_end = remaining;
                    let n = stream.read(&mut buf[buf_end..]).await?;
                    if n == 0 {
                        info!("[S7] --- CLOSE (EOF) from {}", addr);
                        return Ok(());
                    }
                    buf_end += n;
                    continue;
                }
                break;
            }
            
            // Accept TPKT v3 (0x03)
            if buf[buf_start] != 0x03 {
                warn!("[S7] Unknown TPKT: {:#x}, closing", buf[buf_start]);
                return Ok(());
            }
            
            let tpkt_len = ((buf[buf_start + 2] as usize) << 8) | (buf[buf_start + 3] as usize);
            let payload_len = tpkt_len - 4;  // Subtract TPKT header
            buf_start += 4;
            
            // Read TPKT payload
            loop {
                if buf_start + payload_len > buf_end {
                    let remaining = buf_end - buf_start;
                    buf.copy_within(buf_start..buf_end, 0);
                    buf_start = 0;
                    buf_end = remaining;
                    let n = stream.read(&mut buf[buf_end..]).await?;
                    if n == 0 {
                        info!("[S7] --- CLOSE (EOF) from {}", addr);
                        return Ok(());
                    }
                    buf_end += n;
                    continue;
                }
                break;
            }
            
            let payload = &buf[buf_start..buf_start + payload_len];
            buf_start += payload_len;
            
            // Parse payload (could be COTP DT, S7 packet, or mixed)
            self.process_payload(&payload, &addr, &mut stream).await?;
        }
    }
    
    /// Process a TPKT payload (may contain COTP DT and/or S7 packets)
    async fn process_payload(&self, payload: &[u8], addr: &std::net::SocketAddr, stream: &mut tokio::net::TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
                    let tpkt_resp: Vec<u8> = vec![
                        0x03, 0x00,
                        ((resp.len() + 4) >> 8) as u8, ((resp.len() + 4) & 0xFF) as u8,
                    ];
                    stream.write_all(&tpkt_resp).await?;
                    stream.write_all(&resp).await?;
                    stream.flush().await?;
                    debug!("[S7] Sent response: {} bytes", resp.len());
                }
            } else {
                // COTP packet (DT or other)
                // COTP DT: 0x02 0xF0 0x80 ...
                offset += 1; // Skip COTP byte
                // Just consume and ignore COTP for now
                debug!("[S7] Ignoring COTP byte");
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
