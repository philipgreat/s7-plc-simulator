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
        
        // Receive COTP CR
        let mut cotp_buf = vec![0u8; 50];
        let n = stream.read(&mut cotp_buf).await?;
        debug!("COTP CR: {} bytes", n);
        
        // Send COTP CC (Connection Confirm)
        let cotp_cc = vec![
            0x0D, // CC
            0x00, 0x1A, // Length
            0x00, 0x00, // Dest ref
            0x00, 0x01, // Src ref
            0x00, // Class
            0xC0, 0x01, // TPDU size
            0xC1, 0x02, 0x00, 0x00, // Calling TSAP
            0xC2, 0x02, 0x00, 0x00, // Called TSAP
        ];
        stream.write_all(&cotp_cc).await?;
        debug!("Sent COTP CC");
        
        // Send COTP DT (activate)
        stream.write_all(&[0x02, 0xF0, 0x80]).await?;
        debug!("Sent COTP DT");
        
        // Main request loop
        loop {
            // Read S7 header (10 bytes)
            let mut header = [0u8; 10];
            match stream.read_exact(&mut header).await {
                Ok(_) => {}
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        info!("[S7] --- CLOSE (EOF) from {}", addr);
                        return Ok(());
                    }
                    info!("[S7] --- CLOSE (error) from {}: {}", addr, e);
                    return Err(Box::new(e));
                }
            }
            
            // Validate protocol ID
            if header[0] != PROTOCOL_ID {
                warn!("Invalid protocol ID: {:#x}", header[0]);
                continue;
            }
            
            // Parse header
            let pdu_type = header[1];
            let pdu_ref = ((header[4] as u16) << 8) | (header[5] as u16);
            let param_len = ((header[6] as u16) << 8) | (header[7] as u16);
            let data_len = ((header[8] as u16) << 8) | (header[9] as u16);
            
            // Read parameter and data
            let mut param_data = vec![0u8; param_len as usize + data_len as usize];
            if !param_data.is_empty() {
                stream.read_exact(&mut param_data).await?;
            }
            
            // Handle request
            let response = self.handle_request(pdu_type, pdu_ref, &param_data).await;
            
            // Send response
            if let Some(resp) = response {
                stream.write_all(&resp).await?;
                debug!("Sent response: {} bytes", resp.len());
            }
        }
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
                        self.handle_read_request(pdu_ref, &param_data[1..]).await
                    }
                    0x05 => { // Write
                        debug!("Write request");
                        self.handle_write_request(pdu_ref, &param_data[1..]).await
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
    async fn handle_read_request(&self, pdu_ref: u16, params: &[u8]) -> Option<Vec<u8>> {
        if params.len() < 2 {
            return None;
        }
        
        let item_count = params[1] as usize;
        let memory = self.memory.read().ok()?;
        
        let mut response_data = Vec::new();
        let mut offset = 2;
        let mut read_items = Vec::new();
        
        for _ in 0..item_count {
            if offset + 2 > params.len() { break; }
            
            let var_spec = params[offset];
            let var_len = params[offset + 1] as usize;
            
            if var_spec != 0x12 || var_len < 10 || offset + 2 + var_len > params.len() {
                offset += 2;
                continue;
            }
            
            let _transport = params[offset + 3];
            let length = ((params[offset + 5] as u16) << 8) | (params[offset + 6] as u16);
            let db_num = ((params[offset + 7] as u16) << 8) | (params[offset + 8] as u16);
            let area_code = params[offset + 9];
            let start = ((params[offset + 10] as u16) << 8) | (params[offset + 11] as u16);
            
            let area = MemoryArea::from_byte(area_code);
            
            if let Some(area) = area {
                match memory.read(area, db_num, start as usize, length as usize) {
                    Some(data) => {
                        response_data.push(0xFF);
                        response_data.push(TransportSize::Byte as u8);
                        response_data.push((length >> 8) as u8);
                        response_data.push((length & 0xFF) as u8);
                        response_data.extend(data.clone());
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        read_items.push(format!("{:?}[{}] offset={} len={} data=[{}]",
                            area, db_num, start, length, hex_str));
                    }
                    None => {
                        response_data.push(0x0A);
                        response_data.push(0x00);
                        response_data.push(0x00);
                        read_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, length));
                    }
                }
            } else {
                response_data.push(0x0A);
                response_data.push(0x00);
                response_data.push(0x00);
            }
            
            offset += 2 + var_len;
        }
        
        let mut response = vec![
            PROTOCOL_ID, PduType::AckData as u8, 0x00, 0x00,
            (pdu_ref >> 8) as u8, (pdu_ref & 0xFF) as u8,
            0x00, (item_count as u16 + 2) as u8, // Param len
            (response_data.len() >> 8) as u8, (response_data.len() & 0xFF) as u8, // Data len
            FunctionCode::Read as u8, item_count as u8,
        ];
        response.extend(response_data);
        
        if !read_items.is_empty() {
            info!("[READ] items={} {}", read_items.len(), read_items.join(" | "));
        }
        
        Some(response)
    }
    
    /// Handle write request
    async fn handle_write_request(&self, pdu_ref: u16, params: &[u8]) -> Option<Vec<u8>> {
        if params.len() < 2 {
            return None;
        }
        
        let item_count = params[1] as usize;
        let mut memory = self.memory.write().ok()?;
        
        let mut offset = 2;
        for _ in 0..item_count {
            if offset + 2 > params.len() { break; }
            let var_len = params[offset + 1] as usize;
            offset += 2 + var_len;
        }
        
        let mut success_count = 0;
        offset = 2;
        let mut write_items = Vec::new();
        
        for _ in 0..item_count {
            if offset + 2 > params.len() { break; }
            
            let var_spec = params[offset];
            let var_len = params[offset + 1] as usize;
            
            if var_spec != 0x12 || var_len < 10 || offset + 2 + var_len > params.len() {
                offset += 2;
                continue;
            }
            
            let length = ((params[offset + 5] as u16) << 8) | (params[offset + 6] as u16);
            let db_num = ((params[offset + 7] as u16) << 8) | (params[offset + 8] as u16);
            let area_code = params[offset + 9];
            let start = ((params[offset + 10] as u16) << 8) | (params[offset + 11] as u16);
            
            let area = MemoryArea::from_byte(area_code);
            
            if let Some(area) = area {
                let param_end = offset + 2 + var_len;
                if param_end + length as usize <= params.len() {
                    let data = &params[param_end..param_end + length as usize];
                    if memory.write(area, db_num, start as usize, data) {
                        success_count += 1;
                        let hex_str = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        write_items.push(format!("{:?}[{}] offset={} len={} data=[{}] OK",
                            area, db_num, start, length, hex_str));
                    } else {
                        write_items.push(format!("{:?}[{}] offset={} len={} FAIL",
                            area, db_num, start, length));
                    }
                }
            }
            
            offset += 2 + var_len;
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
