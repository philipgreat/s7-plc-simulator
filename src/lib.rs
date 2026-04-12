//!
//! S7 PLC Simulator
//! 
//! A simulated S7 PLC that maintains internal state for testing S7 Connector

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

// (BufMut removed - not used directly)

/// Memory area type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryArea {
    Inputs = 0x81,
    Outputs = 0x82,
    Flags = 0x83,
    DataBlocks = 0x84,
    Counters = 0x1C,
    Timers = 0x1D,
}

/// Protocol ID
const PROTOCOL_ID: u8 = 0x32;

/// PDU types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PduType {
    Job = 0x01,
    Ack = 0x02,
    AckData = 0x03,
    UserData = 0x07,
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

/// Simulated PLC memory
#[derive(Debug, Default)]
pub struct PlcMemory {
    /// Input markers (I)
    inputs: Vec<u8>,
    /// Output markers (Q)
    outputs: Vec<u8>,
    /// Memory markers (M)
    flags: Vec<u8>,
    /// Data blocks (DB)
    data_blocks: HashMap<u16, Vec<u8>>,
    /// Counters
    counters: Vec<u16>,
    /// Timers
    timers: Vec<u32>,
}

impl PlcMemory {
    /// Create new PLC memory with default size
    pub fn new() -> Self {
        Self {
            inputs: vec![0u8; 256],
            outputs: vec![0u8; 256],
            flags: vec![0u8; 1024],
            data_blocks: HashMap::new(),
            counters: vec![0u16; 64],
            timers: vec![0u32; 64],
        }
    }
    
    /// Initialize default data blocks
    pub fn init_default_db(&mut self) {
        // DB1 - General data
        self.data_blocks.insert(1, vec![0u8; 256]);
        // DB2 - Counter values
        self.data_blocks.insert(2, vec![0u8; 128]);
        // DB3 - Timer values
        self.data_blocks.insert(3, vec![0u8; 128]);
        // DB10 - Real values
        let mut db10 = vec![0u8; 64];
        // Put some test float values
        let test_floats: [f32; 4] = [1.5, 2.5, 3.14, 100.0];
        for (i, &f) in test_floats.iter().enumerate() {
            let bytes = f.to_be_bytes();
            db10[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
        }
        self.data_blocks.insert(10, db10);
        // DB11 - Integer values
        let mut db11 = vec![0u8; 32];
        let test_ints: [i16; 8] = [100, -200, 300, -400, 500, -600, 700, -800];
        for (i, &v) in test_ints.iter().enumerate() {
            db11[i * 2..(i + 1) * 2].copy_from_slice(&v.to_be_bytes());
        }
        self.data_blocks.insert(11, db11);
        // DB20 - String test
        let mut db20 = vec![0u8; 128];
        // String header: max len (2 bytes) + actual len (2 bytes)
        db20[0] = 0xFF;
        db20[1] = 0xFE;
        db20[2] = 0x00;
        db20[3] = 0x0C; // 12 characters
        // "Hello World!" as ASCII
        "Hello World!".as_bytes().iter().enumerate().for_each(|(i, &b)| {
            db20[4 + i] = b;
        });
        self.data_blocks.insert(20, db20);
    }
    
    /// Read from memory area
    pub fn read(&self, area: MemoryArea, db_num: u16, start: usize, len: usize) -> Option<Vec<u8>> {
        match area {
            MemoryArea::Inputs => {
                if start + len <= self.inputs.len() {
                    Some(self.inputs[start..start + len].to_vec())
                } else {
                    None
                }
            }
            MemoryArea::Outputs => {
                if start + len <= self.outputs.len() {
                    Some(self.outputs[start..start + len].to_vec())
                } else {
                    None
                }
            }
            MemoryArea::Flags => {
                if start + len <= self.flags.len() {
                    Some(self.flags[start..start + len].to_vec())
                } else {
                    None
                }
            }
            MemoryArea::DataBlocks => {
                self.data_blocks.get(&db_num).and_then(|db| {
                    if start + len <= db.len() {
                        Some(db[start..start + len].to_vec())
                    } else {
                        None
                    }
                })
            }
            _ => None,
        }
    }
    
    /// Write to memory area
    pub fn write(&mut self, area: MemoryArea, db_num: u16, start: usize, data: &[u8]) -> bool {
        match area {
            MemoryArea::Inputs => {
                if start + data.len() <= self.inputs.len() {
                    self.inputs[start..start + data.len()].copy_from_slice(data);
                    true
                } else {
                    false
                }
            }
            MemoryArea::Outputs => {
                if start + data.len() <= self.outputs.len() {
                    self.outputs[start..start + data.len()].copy_from_slice(data);
                    true
                } else {
                    false
                }
            }
            MemoryArea::Flags => {
                if start + data.len() <= self.flags.len() {
                    self.flags[start..start + data.len()].copy_from_slice(data);
                    true
                } else {
                    false
                }
            }
            MemoryArea::DataBlocks => {
                if let Some(db) = self.data_blocks.get_mut(&db_num) {
                    if start + data.len() <= db.len() {
                        db[start..start + data.len()].copy_from_slice(data);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    
    /// Get DB size
    pub fn get_db_size(&self, db_num: u16) -> Option<usize> {
        self.data_blocks.get(&db_num).map(|db| db.len())
    }
}

/// S7 PLC Simulator
pub struct PlcSimulator {
    /// PLC memory
    memory: Arc<RwLock<PlcMemory>>,
    /// PLC type
    plc_type: String,
    /// Rack and slot
    rack: u8,
    slot: u8,
    /// Serial number
    serial_number: String,
    /// Module name
    module_name: String,
    /// PDU reference counter
    pdu_ref: u16,
}

impl PlcSimulator {
    /// Create new PLC simulator
    pub fn new(plc_type: &str, rack: u8, slot: u8) -> Self {
        let mut memory = PlcMemory::new();
        memory.init_default_db();
        
        Self {
            memory: Arc::new(RwLock::new(memory)),
            plc_type: plc_type.to_string(),
            rack,
            slot,
            serial_number: "SIM001".to_string(),
            module_name: format!("CPU {}", plc_type),
            pdu_ref: 1,
        }
    }
    
    /// Get next PDU reference
    fn next_pdu_ref(&mut self) -> u16 {
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        self.pdu_ref
    }
    
    /// Get PLC info
    pub fn get_plc_info(&self) -> PlcInfo {
        PlcInfo {
            plc_type: self.plc_type.clone(),
            rack: self.rack,
            slot: self.slot,
            serial_number: self.serial_number.clone(),
            module_name: self.module_name.clone(),
        }
    }
    
    /// Handle incoming connection
    pub async fn handle_connection(&mut self, stream: TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = stream.peer_addr()?;
        info!("New connection from: {}", addr);
        
        // Clone memory Arc for the task
        let memory = self.memory.clone();
        
        let mut stream = stream;
        
        // Receive COTP CR
        let mut cotp_buf = vec![0u8; 50];
        let n = stream.read(&mut cotp_buf).await?;
        debug!("Received COTP CR: {:?} ({} bytes)", &cotp_buf[..n.min(20)], n);
        
        // Send COTP CC (Connection Confirm)
        let cotp_cc = vec![
            0x0D, // CC (Connection Confirm)
            0x00, 0x1A, // Length
            0x00, 0x00, // Destination reference
            0x00, 0x01, // Source reference
            0x00, // Class
            0xC0, // TPDU size indicator
            0x01, // TPDU size: 256 bytes
            0xC1, // Calling TSAP present
            0x02, // Length 2
            0x00, 0x00,
            0xC2, // Called TSAP present
            0x02, // Length 2
            0x00, // PLC type
            0x00, // Rack/Slot
        ];
        
        stream.write_all(&cotp_cc).await?;
        debug!("Sent COTP CC");
        
        // Send COTP DT (activate)
        let dt_activate = vec![0x02, 0xF0, 0x80];
        stream.write_all(&dt_activate).await?;
        debug!("Sent COTP DT activate");
        
        // Main request loop
        loop {
            // Read S7 header (10 bytes)
            let mut header = [0u8; 10];
            match stream.read_exact(&mut header).await {
                Ok(_) => {}
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        info!("Client disconnected: {}", addr);
                        return Ok(());
                    }
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
            let _reserved = ((header[2] as u16) << 8) | (header[3] as u16);
            let pdu_ref = ((header[4] as u16) << 8) | (header[5] as u16);
            let param_len = ((header[6] as u16) << 8) | (header[7] as u16);
            let data_len = ((header[8] as u16) << 8) | (header[9] as u16);
            
            debug!("PDU: type={:#x}, ref={}, param_len={}, data_len={}", 
                   pdu_type, pdu_ref, param_len, data_len);
            
            // Read parameter and data
            let mut param_data = vec![0u8; param_len as usize + data_len as usize];
            if !param_data.is_empty() {
                stream.read_exact(&mut param_data).await?;
            }
            
            // Handle request
            let response = self.handle_request(pdu_type, pdu_ref, &param_data, memory.clone()).await;
            
            // Send response
            if let Some(resp) = response {
                stream.write_all(&resp).await?;
                debug!("Sent response: {} bytes", resp.len());
            }
        }
    }
    
    /// Handle S7 request
    async fn handle_request(&mut self, pdu_type: u8, pdu_ref: u16, param_data: &[u8], memory: Arc<RwLock<PlcMemory>>) -> Option<Vec<u8>> {
        // Handle different PDU types
        match pdu_type {
            0x01 => { // Job (request)
                if param_data.len() < 1 {
                    return None;
                }
                
                let function_code = param_data[0];
                
                match function_code {
                    0xF0 => { // Setup Communication
                        debug!("Setup Communication request");
                        Some(Self::build_setup_response(pdu_ref))
                    }
                    0x04 => { // Read
                        debug!("Read request");
                        self.handle_read_request(pdu_ref, &param_data[1..], memory).await
                    }
                    0x05 => { // Write
                        debug!("Write request");
                        self.handle_write_request(pdu_ref, &param_data[1..], memory).await
                    }
                    _ => {
                        warn!("Unsupported function code: {:#x}", function_code);
                        None
                    }
                }
            }
            0x07 => { // User data
                debug!("User data PDU");
                Some(Self::build_user_data_response(pdu_ref))
            }
            _ => {
                warn!("Unknown PDU type: {:#x}", pdu_type);
                None
            }
        }
    }
    
    /// Build setup communication response
    fn build_setup_response(pdu_ref: u16) -> Vec<u8> {
        let mut response = Vec::with_capacity(32);
        
        // Header
        response.push(PROTOCOL_ID);
        response.push(PduType::AckData as u8);
        response.push(0x00); // Reserved high
        response.push(0x00); // Reserved low
        response.push((pdu_ref >> 8) as u8); // Reference high
        response.push((pdu_ref & 0xFF) as u8); // Reference low
        response.push(0x00); // Param len high
        response.push(0x08); // Param len low (8 bytes)
        response.push(0x00); // Data len high
        response.push(0x00); // Data len low
        
        // Parameters (Setup Communication response)
        response.push(FunctionCode::SetupCommunication as u8);
        response.push(0x00);
        response.push(0x03); // Max AmQ BE (1000) high
        response.push(0xE8); // Max AmQ BE low
        response.push(0x03); // Max AmQ LE (1000) high
        response.push(0xE8); // Max AmQ LE low
        response.push(0x01); // PDU length (480) high
        response.push(0xE0); // PDU length low
        
        response
    }
    
    /// Handle read request
    async fn handle_read_request(&self, pdu_ref: u16, params: &[u8], memory: Arc<RwLock<PlcMemory>>) -> Option<Vec<u8>> {
        if params.len() < 2 {
            return None;
        }
        
        let item_count = params[1] as usize;
        
        // Read all request items
        let mut request_items = Vec::new();
        let mut offset = 2;
        
        for _ in 0..item_count {
            if offset + 2 > params.len() {
                break;
            }
            
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
            
            request_items.push((area_code, db_num, start, length as usize));
            
            offset += 2 + var_len;
        }
        
        // Read data
        let memory = memory.read().ok()?;
        let mut response_data = Vec::new();
        
        for (area_code, db_num, start, length) in request_items {
            let area = match area_code {
                0x81 => Some(MemoryArea::Inputs),
                0x82 => Some(MemoryArea::Outputs),
                0x83 => Some(MemoryArea::Flags),
                0x84 => Some(MemoryArea::DataBlocks),
                _ => None,
            };
            
            if let Some(area) = area {
                match memory.read(area, db_num, start as usize, length) {
                    Some(data) => {
                        response_data.push(0xFF); // Success
                        response_data.push(TransportSize::Byte as u8);
                        response_data.push((length >> 8) as u8);
                        response_data.push((length & 0xFF) as u8);
                        response_data.extend(data);
                    }
                    None => {
                        response_data.push(0x0A); // Error: address error
                        response_data.push(0x00);
                        response_data.push(0x00);
                    }
                }
            } else {
                response_data.push(0x0A); // Address error
                response_data.push(0x00);
                response_data.push(0x00);
            }
        }
        
        // Build response
        let mut response = Vec::with_capacity(32 + response_data.len());
        
        // Header
        response.push(PROTOCOL_ID);
        response.push(PduType::AckData as u8);
        response.push(0x00); // Reserved high
        response.push(0x00); // Reserved low
        response.push((pdu_ref >> 8) as u8); // Reference high
        response.push((pdu_ref & 0xFF) as u8); // Reference low
        response.push(0x00); // Param len high
        response.push((item_count as u16 + 2) as u8); // Param len low
        response.push((response_data.len() >> 8) as u8); // Data len high
        response.push((response_data.len() & 0xFF) as u8); // Data len low
        
        // Parameters
        response.push(FunctionCode::Read as u8);
        response.push(item_count as u8);
        
        // Data
        response.extend(response_data);
        
        Some(response)
    }
    
    /// Handle write request
    async fn handle_write_request(&self, pdu_ref: u16, params: &[u8], memory: Arc<RwLock<PlcMemory>>) -> Option<Vec<u8>> {
        if params.len() < 2 {
            return None;
        }
        
        let item_count = params[1] as usize;
        
        // Calculate data offset
        let mut offset = 2;
        for _ in 0..item_count {
            if offset + 2 > params.len() {
                break;
            }
            let _var_spec = params[offset];
            let var_len = params[offset + 1] as usize;
            offset += 2 + var_len;
        }
        
        // Process write requests
        let mut memory = match memory.write() {
            Ok(m) => m,
            Err(_) => return None,
        };
        
        let mut success_count = 0;
        offset = 2;
        
        for _ in 0..item_count {
            if offset + 2 > params.len() {
                break;
            }
            
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
            
            let area = match area_code {
                0x81 => Some(MemoryArea::Inputs),
                0x82 => Some(MemoryArea::Outputs),
                0x83 => Some(MemoryArea::Flags),
                0x84 => Some(MemoryArea::DataBlocks),
                _ => None,
            };
            
            if let Some(area) = area {
                // Read data from PDU
                let param_end = offset + 2 + var_len;
                if param_end + length as usize <= params.len() {
                    let data = &params[param_end..param_end + length as usize];
                    if memory.write(area, db_num, start as usize, data) {
                        success_count += 1;
                        debug!("Wrote {} bytes to {:?} DB{}", length, area, db_num);
                    }
                }
            }
            
            offset += 2 + var_len;
        }
        
        // Build response
        let mut response = Vec::with_capacity(16);
        
        // Header
        response.push(PROTOCOL_ID);
        response.push(PduType::AckData as u8);
        response.push(0x00); // Reserved high
        response.push(0x00); // Reserved low
        response.push((pdu_ref >> 8) as u8); // Reference high
        response.push((pdu_ref & 0xFF) as u8); // Reference low
        response.push(0x00); // Param len high
        response.push(0x02); // Param len low
        response.push(0x00); // Data len high
        response.push(item_count as u8); // Data len low
        
        // Parameters
        response.push(FunctionCode::Write as u8);
        response.push(item_count as u8);
        
        // Data (return codes)
        for _ in 0..item_count {
            response.push(0xFF); // Success
        }
        
        info!("Write: {}/{} items successful", success_count, item_count);
        
        Some(response)
    }
    
    /// Build user data response
    fn build_user_data_response(pdu_ref: u16) -> Vec<u8> {
        let mut response = Vec::with_capacity(16);
        
        // Header
        response.push(PROTOCOL_ID);
        response.push(PduType::AckData as u8);
        response.push(0x00); // Reserved high
        response.push(0x00); // Reserved low
        response.push((pdu_ref >> 8) as u8); // Reference high
        response.push((pdu_ref & 0xFF) as u8); // Reference low
        response.push(0x00); // Param len high
        response.push(0x04); // Param len low
        response.push(0x00); // Data len high
        response.push(0x00); // Data len low
        
        // Parameters
        response.push(0x00);
        response.push(0x00);
        response.push(0x00);
        response.push(0x00);
        
        response
    }
    
    /// Start the PLC simulator server
    pub async fn start_server(port: u16, plc_type: &str, rack: u8, slot: u8) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!("0.0.0.0:{}", port);
        info!("Starting S7 PLC Simulator on {}", addr);
        
        let listener = TcpListener::bind(&addr).await?;
        info!("S7 PLC Simulator listening on {}", addr);
        
        let plc_type = plc_type.to_string();
        
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let mut simulator = PlcSimulator::new(&plc_type, rack, slot);
                    info!("New connection from: {}", addr);
                    
                    tokio::spawn(async move {
                        if let Err(e) = simulator.handle_connection(stream).await {
                            error!("Connection handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

/// PLC information
#[derive(Debug, Clone)]
pub struct PlcInfo {
    pub plc_type: String,
    pub rack: u8,
    pub slot: u8,
    pub serial_number: String,
    pub module_name: String,
}
