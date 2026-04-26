//!
//! PLC Memory Management
//! 
//! Manages memory areas and data blocks for the simulated PLC

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::fs;
use std::path::Path;
use chrono::{Datelike, Timelike};

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

impl MemoryArea {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x81 => Some(Self::Inputs),
            0x82 => Some(Self::Outputs),
            0x83 => Some(Self::Flags),
            0x84 => Some(Self::DataBlocks),
            0x1C => Some(Self::Counters),
            0x1D => Some(Self::Timers),
            _ => None,
        }
    }
    
    pub fn name(&self) -> &'static str {
        match self {
            Self::Inputs => "Inputs",
            Self::Outputs => "Outputs",
            Self::Flags => "Flags",
            Self::DataBlocks => "DataBlocks",
            Self::Counters => "Counters",
            Self::Timers => "Timers",
        }
    }
}

/// Data type definition for a memory variable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDefinition {
    pub name: String,
    pub offset: usize,
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub enum_values: Option<HashMap<String, String>>,
}

/// Data block configuration from JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlockConfig {
    pub number: u16,
    pub size: usize,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub variables: Vec<VariableDefinition>,
}

/// PLC configuration from JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlcConfig {
    pub plc: PlcInfo,
    pub memory: MemoryConfig,
    pub data_blocks: Vec<DataBlockConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlcInfo {
    #[serde(rename = "type")]
    pub plc_type: String,
    pub rack: u8,
    pub slot: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub inputs: AreaConfig,
    pub outputs: AreaConfig,
    pub flags: AreaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreaConfig {
    pub size: usize,
}

/// Data block definition for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlock {
    pub number: u16,
    pub size: usize,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub variables: Vec<VariableDefinition>,
}

/// Data block with raw bytes
#[derive(Debug, Clone)]
pub struct DataBlockData {
    pub number: u16,
    pub bytes: Vec<u8>,
    pub description: Option<String>,
    pub variables: Vec<VariableDefinition>,
}

impl DataBlockData {
    pub fn new(number: u16, size: usize) -> Self {
        Self {
            number,
            bytes: vec![0u8; size],
            description: None,
            variables: Vec::new(),
        }
    }
    
    pub fn with_config(config: &DataBlockConfig) -> Self {
        let mut db = Self {
            number: config.number,
            bytes: vec![0u8; config.size],
            description: config.description.clone(),
            variables: config.variables.clone(),
        };
        db.init_from_variables();
        db
    }
    
    /// Initialize bytes from variable definitions
    fn init_from_variables(&mut self) {
        let variables = self.variables.clone();
        for var in &variables {
            self.set_variable_value(var);
        }
    }
    
    /// Set variable value from definition
    fn set_variable_value(&mut self, var: &VariableDefinition) {
        let offset = var.offset;
        
        match var.data_type.to_uppercase().as_str() {
            "BOOL" => {
                if let Some(v) = var.value.as_bool() {
                    if offset < self.bytes.len() {
                        self.bytes[offset] = if v { 0x01 } else { 0x00 };
                    }
                }
            }
            "BYTE" => {
                if let Some(v) = var.value.as_u64() {
                    if offset < self.bytes.len() {
                        self.bytes[offset] = v as u8;
                    }
                }
            }
            "WORD" => {
                if let Some(v) = var.value.as_u64() {
                    if offset + 1 < self.bytes.len() {
                        self.bytes[offset] = (v >> 8) as u8;
                        self.bytes[offset + 1] = (v & 0xFF) as u8;
                    }
                }
            }
            "DWORD" => {
                if let Some(v) = var.value.as_u64() {
                    if offset + 3 < self.bytes.len() {
                        self.bytes[offset] = (v >> 24) as u8;
                        self.bytes[offset + 1] = ((v >> 16) & 0xFF) as u8;
                        self.bytes[offset + 2] = ((v >> 8) & 0xFF) as u8;
                        self.bytes[offset + 3] = (v & 0xFF) as u8;
                    }
                }
            }
            "INT" => {
                if let Some(v) = var.value.as_i64() {
                    if offset + 1 < self.bytes.len() {
                        let val = v as i16;
                        self.bytes[offset] = (val >> 8) as u8;
                        self.bytes[offset + 1] = (val & 0xFF) as u8;
                    }
                }
            }
            "DINT" => {
                if let Some(v) = var.value.as_i64() {
                    if offset + 3 < self.bytes.len() {
                        let val = v as i32;
                        self.bytes[offset] = (val >> 24) as u8;
                        self.bytes[offset + 1] = ((val >> 16) & 0xFF) as u8;
                        self.bytes[offset + 2] = ((val >> 8) & 0xFF) as u8;
                        self.bytes[offset + 3] = (val & 0xFF) as u8;
                    }
                }
            }
            "REAL" => {
                if let Some(v) = var.value.as_f64() {
                    if offset + 3 < self.bytes.len() {
                        let bytes = (v as f32).to_be_bytes();
                        self.bytes[offset..offset + 4].copy_from_slice(&bytes);
                    }
                }
            }
            "STRING" => {
                if let Some(s) = var.value.as_str() {
                    let max_len = var.max_length.unwrap_or(254);
                    let actual_len = s.len().min(max_len);
                    if offset + 4 + actual_len <= self.bytes.len() {
                        // S7 String format: [max_len_hi][max_len_lo][actual_len_hi][actual_len_lo][data...]
                        self.bytes[offset] = 0xFF;
                        self.bytes[offset + 1] = 0xFE;
                        self.bytes[offset + 2] = 0x00;
                        self.bytes[offset + 3] = actual_len as u8;
                        self.bytes[offset + 4..offset + 4 + actual_len].copy_from_slice(s.as_bytes());
                    }
                }
            }
            "DT" | "DATE_AND_TIME" => {
                // S7 DateAndTime format: BCD encoded
                if let Some(s) = var.value.as_str() {
                    // Parse RFC3339 or simple format
                    let dt_str = if s.contains('T') {
                        s.to_string()
                    } else {
                        format!("{}T00:00:00Z", s)
                    };
                    
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&dt_str) {
                        if offset + 7 < self.bytes.len() {
                            let year = dt.year();
                            self.bytes[offset] = ((year % 100) / 10 * 16 + year % 10) as u8; // BCD year
                            self.bytes[offset + 1] = ((dt.month() / 10) * 16 + dt.month() % 10) as u8; // BCD month
                            self.bytes[offset + 2] = ((dt.day() / 10) * 16 + dt.day() % 10) as u8; // BCD day
                            self.bytes[offset + 3] = ((dt.hour() / 10) * 16 + dt.hour() % 10) as u8; // BCD hour
                            self.bytes[offset + 4] = ((dt.minute() / 10) * 16 + dt.minute() % 10) as u8; // BCD minute
                            self.bytes[offset + 5] = ((dt.second() / 10) * 16 + dt.second() % 10) as u8; // BCD second
                            self.bytes[offset + 6] = 0x07; // Day of week (1=Sunday..7=Saturday)
                            self.bytes[offset + 7] = 0x00; // msec high nibble
                        }
                    }
                }
            }
            _ => {}
        }
    }
    
    /// Get variable value as JSON
    pub fn get_variable_value(&self, var: &VariableDefinition) -> serde_json::Value {
        let offset = var.offset;
        
        match var.data_type.to_uppercase().as_str() {
            "BOOL" => {
                if offset < self.bytes.len() {
                    serde_json::json!(self.bytes[offset] != 0)
                } else {
                    serde_json::json!(null)
                }
            }
            "BYTE" => {
                if offset < self.bytes.len() {
                    serde_json::json!(self.bytes[offset])
                } else {
                    serde_json::json!(null)
                }
            }
            "WORD" => {
                if offset + 1 < self.bytes.len() {
                    let val = ((self.bytes[offset] as u16) << 8) | (self.bytes[offset + 1] as u16);
                    serde_json::json!(val)
                } else {
                    serde_json::json!(null)
                }
            }
            "DWORD" => {
                if offset + 3 < self.bytes.len() {
                    let val = ((self.bytes[offset] as u32) << 24)
                        | ((self.bytes[offset + 1] as u32) << 16)
                        | ((self.bytes[offset + 2] as u32) << 8)
                        | (self.bytes[offset + 3] as u32);
                    serde_json::json!(val)
                } else {
                    serde_json::json!(null)
                }
            }
            "INT" => {
                if offset + 1 < self.bytes.len() {
                    let val = i16::from_be_bytes([self.bytes[offset], self.bytes[offset + 1]]);
                    serde_json::json!(val)
                } else {
                    serde_json::json!(null)
                }
            }
            "DINT" => {
                if offset + 3 < self.bytes.len() {
                    let val = i32::from_be_bytes([
                        self.bytes[offset],
                        self.bytes[offset + 1],
                        self.bytes[offset + 2],
                        self.bytes[offset + 3],
                    ]);
                    serde_json::json!(val)
                } else {
                    serde_json::json!(null)
                }
            }
            "REAL" => {
                if offset + 3 < self.bytes.len() {
                    let val = f32::from_be_bytes([
                        self.bytes[offset],
                        self.bytes[offset + 1],
                        self.bytes[offset + 2],
                        self.bytes[offset + 3],
                    ]);
                    serde_json::json!(val)
                } else {
                    serde_json::json!(null)
                }
            }
            "STRING" => {
                if offset + 3 < self.bytes.len() {
                    let actual_len = self.bytes[offset + 3] as usize;
                    if offset + 4 + actual_len <= self.bytes.len() {
                        if let Ok(s) = std::str::from_utf8(&self.bytes[offset + 4..offset + 4 + actual_len]) {
                            return serde_json::json!(s);
                        }
                    }
                }
                serde_json::json!(null)
            }
            _ => serde_json::json!(null),
        }
    }
}

/// PLC Memory
#[derive(Debug, Default)]
pub struct PlcMemory {
    inputs: Vec<u8>,
    outputs: Vec<u8>,
    flags: Vec<u8>,
    data_blocks: HashMap<u16, DataBlockData>,
    counters: Vec<u16>,
    timers: Vec<u32>,
}

impl PlcMemory {
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
    
    /// Load configuration from JSON file
    pub fn from_config_file(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        
        let config: PlcConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config JSON: {}", e))?;
        
        let mut memory = Self {
            inputs: vec![0u8; config.memory.inputs.size],
            outputs: vec![0u8; config.memory.outputs.size],
            flags: vec![0u8; config.memory.flags.size],
            data_blocks: HashMap::new(),
            counters: vec![0u16; 64],
            timers: vec![0u32; 64],
        };
        
        // Initialize data blocks from config
        for db_config in &config.data_blocks {
            let db = DataBlockData::with_config(db_config);
            memory.data_blocks.insert(db.number, db);
        }
        
        Ok(memory)
    }
    
    /// Initialize default data blocks (fallback if no config)
    pub fn init_default_db(&mut self) {
        self.add_db(1, 256);
        self.add_db(2, 128);
        self.add_db(3, 128);
        
        // DB10 - Real values
        let mut db10 = DataBlockData::new(10, 64);
        let test_floats: [f32; 4] = [1.5, 2.5, 3.14, 100.0];
        for (i, &f) in test_floats.iter().enumerate() {
            let offset = i * 4;
            db10.bytes[offset..offset + 4].copy_from_slice(&f.to_be_bytes());
        }
        self.data_blocks.insert(10, db10);
        
        // DB11 - Integer values
        let mut db11 = DataBlockData::new(11, 32);
        let test_ints: [i16; 8] = [100, -200, 300, -400, 500, -600, 700, -800];
        for (i, &v) in test_ints.iter().enumerate() {
            let offset = i * 2;
            db11.bytes[offset..offset + 2].copy_from_slice(&v.to_be_bytes());
        }
        self.data_blocks.insert(11, db11);
        
        // DB20 - String test
        let mut db20 = DataBlockData::new(20, 128);
        db20.bytes[0] = 0xFF; // Max length high
        db20.bytes[1] = 0xFE; // Max length low
        db20.bytes[2] = 0x00;
        db20.bytes[3] = 0x0C; // Actual length
        "Hello World!".as_bytes().iter().enumerate().for_each(|(i, &b)| {
            db20.bytes[4 + i] = b;
        });
        self.data_blocks.insert(20, db20);
        
        // DB100 - Filling Station Status
        let mut db100 = DataBlockData::new(100, 256);
        
        // status: RUNNING (1)
        db100.bytes[0] = 0x00;
        db100.bytes[1] = 0x00;
        db100.bytes[2] = 0x00;
        db100.bytes[3] = 0x01;
        
        // taskId: 12345
        db100.bytes[4] = 0x00;
        db100.bytes[5] = 0x00;
        db100.bytes[6] = 0x30;
        db100.bytes[7] = 0x39;
        
        // progress: 65.5%
        let progress: f32 = 65.5;
        db100.bytes[8..12].copy_from_slice(&progress.to_be_bytes());
        
        // volumeDispensed: 327.5 L
        let volume: f32 = 327.5;
        db100.bytes[12..16].copy_from_slice(&volume.to_be_bytes());
        
        // targetVolume: 500.0 L
        let target: f32 = 500.0;
        db100.bytes[16..20].copy_from_slice(&target.to_be_bytes());
        
        // flowRate: 45.2 L/min
        let flow: f32 = 45.2;
        db100.bytes[20..24].copy_from_slice(&flow.to_be_bytes());
        
        // pressure: 2.8 bar
        let pressure: f32 = 2.8;
        db100.bytes[24..28].copy_from_slice(&pressure.to_be_bytes());
        
        // temperature: 22.5 ℃
        let temp: f32 = 22.5;
        db100.bytes[28..32].copy_from_slice(&temp.to_be_bytes());
        
        // fillStationId: 1
        db100.bytes[32] = 0x00;
        db100.bytes[33] = 0x00;
        db100.bytes[34] = 0x00;
        db100.bytes[35] = 0x01;
        
        // startTime: now - 120 seconds
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        let start = (now - 120).to_be_bytes();
        db100.bytes[36..40].copy_from_slice(&start);
        
        // endTime: 0 (not ended)
        db100.bytes[40] = 0x00;
        db100.bytes[41] = 0x00;
        db100.bytes[42] = 0x00;
        db100.bytes[43] = 0x00;
        
        // errorCode: 0 (no error)
        db100.bytes[44] = 0x00;
        
        self.data_blocks.insert(100, db100);

        // DB401 - Filling Station Status
        let mut db401 = DataBlockData::new(401, 38);

        // stationStatus: RUNNING (1)
        db401.bytes[0] = 0x00;
        db401.bytes[1] = 0x00;
        db401.bytes[2] = 0x00;
        db401.bytes[3] = 0x01;

        // currentTaskId: 67890
        db401.bytes[4] = 0x00;
        db401.bytes[5] = 0x01;
        db401.bytes[6] = 0x09;
        db401.bytes[7] = 0x32;

        // fillProgress: 72.5%
        let fill_progress: f32 = 72.5;
        db401.bytes[8..12].copy_from_slice(&fill_progress.to_be_bytes());

        // currentVolume: 362.5 L
        let current_vol: f32 = 362.5;
        db401.bytes[12..16].copy_from_slice(&current_vol.to_be_bytes());

        // targetVolume: 500.0 L
        let target_vol: f32 = 500.0;
        db401.bytes[16..20].copy_from_slice(&target_vol.to_be_bytes());

        // flowRate: 42.8 L/min
        let flow_rate: f32 = 42.8;
        db401.bytes[20..24].copy_from_slice(&flow_rate.to_be_bytes());

        // pressure: 3.1 bar
        let press: f32 = 3.1;
        db401.bytes[24..28].copy_from_slice(&press.to_be_bytes());

        // temperature: 23.4 ℃
        let temp401: f32 = 23.4;
        db401.bytes[28..32].copy_from_slice(&temp401.to_be_bytes());

        // elapsedSeconds: 300 (5 minutes)
        db401.bytes[32] = 0x00;
        db401.bytes[33] = 0x00;
        db401.bytes[34] = 0x01;
        db401.bytes[35] = 0x2C;

        // errorCode: 0 (no error)
        db401.bytes[36] = 0x00;
        db401.bytes[37] = 0x00;

        self.data_blocks.insert(401, db401);

        // DB2991 - Filling Station Report
        let mut db2991 = DataBlockData::new(2991, 808);

        // reportCount: 1
        db2991.bytes[0] = 0x00;
        db2991.bytes[1] = 0x00;
        db2991.bytes[2] = 0x00;
        db2991.bytes[3] = 0x01;

        // reportIndex: 0
        db2991.bytes[4] = 0x00;
        db2991.bytes[5] = 0x00;
        db2991.bytes[6] = 0x00;
        db2991.bytes[7] = 0x00;

        // taskId at offset 780: "TASK0001" as ASCII
        let task_id = b"TASK0001";
        db2991.bytes[780..788].copy_from_slice(task_id);

        // startTime at offset 472: S7 DateAndTime BCD format
        db2991.bytes[472] = 0x26; // year (BCD)
        db2991.bytes[473] = 0x04; // month (BCD)
        db2991.bytes[474] = 0x19; // day (BCD)
        db2991.bytes[475] = 0x18; // hour (BCD)
        db2991.bytes[476] = 0x00; // minute (BCD)
        db2991.bytes[477] = 0x00; // second (BCD)
        db2991.bytes[478] = 0x07; // day of week (BCD)
        db2991.bytes[479] = 0x00; // msec high nibble

        // endTime at offset 480
        db2991.bytes[480] = 0x26;
        db2991.bytes[481] = 0x04;
        db2991.bytes[482] = 0x19;
        db2991.bytes[483] = 0x18;
        db2991.bytes[484] = 0x05;
        db2991.bytes[485] = 0x00;
        db2991.bytes[486] = 0x07;
        db2991.bytes[487] = 0x00;

        self.data_blocks.insert(2991, db2991);
    }
    
    /// Add a new data block
    pub fn add_db(&mut self, number: u16, size: usize) {
        self.data_blocks.insert(number, DataBlockData::new(number, size));
    }
    
    /// Remove a data block
    pub fn remove_db(&mut self, number: u16) -> bool {
        self.data_blocks.remove(&number).is_some()
    }
    
    /// Get data block info with variables
    pub fn get_db_info(&self, number: u16) -> Option<DataBlock> {
        self.data_blocks.get(&number).map(|db| DataBlock {
            number: db.number,
            size: db.bytes.len(),
            description: db.description.clone(),
            variables: db.variables.clone(),
        })
    }
    
    /// List all data blocks
    pub fn list_dbs(&self) -> Vec<DataBlock> {
        self.data_blocks.values().map(|db| DataBlock {
            number: db.number,
            size: db.bytes.len(),
            description: db.description.clone(),
            variables: db.variables.clone(),
        }).collect()
    }
    
    /// Get number of data blocks
    pub fn db_count(&self) -> usize {
        self.data_blocks.len()
    }
    
    /// Read bytes from memory area
    pub fn read(&self, area: MemoryArea, db_num: u16, start: usize, len: usize) -> Option<Vec<u8>> {
        match area {
            MemoryArea::Inputs => self.inputs.get(start..start + len).map(|s| s.to_vec()),
            MemoryArea::Outputs => self.outputs.get(start..start + len).map(|s| s.to_vec()),
            MemoryArea::Flags => self.flags.get(start..start + len).map(|s| s.to_vec()),
            MemoryArea::DataBlocks => self.data_blocks.get(&db_num)
                .and_then(|db| db.bytes.get(start..start + len).map(|s| s.to_vec())),
            _ => None,
        }
    }
    
    /// Write bytes to memory area
    pub fn write(&mut self, area: MemoryArea, db_num: u16, start: usize, data: &[u8]) -> bool {
        match area {
            MemoryArea::Inputs if start + data.len() <= self.inputs.len() => {
                self.inputs[start..start + data.len()].copy_from_slice(data);
                true
            }
            MemoryArea::Outputs if start + data.len() <= self.outputs.len() => {
                self.outputs[start..start + data.len()].copy_from_slice(data);
                true
            }
            MemoryArea::Flags if start + data.len() <= self.flags.len() => {
                self.flags[start..start + data.len()].copy_from_slice(data);
                true
            }
            MemoryArea::DataBlocks => {
                if let Some(db) = self.data_blocks.get_mut(&db_num) {
                    if start + data.len() <= db.bytes.len() {
                        db.bytes[start..start + data.len()].copy_from_slice(data);
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    
    /// Read single byte
    pub fn read_byte(&self, area: MemoryArea, db_num: u16, offset: usize) -> Option<u8> {
        self.read(area, db_num, offset, 1).and_then(|v| v.first().copied())
    }
    
    /// Write single byte
    pub fn write_byte(&mut self, area: MemoryArea, db_num: u16, offset: usize, value: u8) -> bool {
        self.write(area, db_num, offset, &[value])
    }
    
    /// Read word (2 bytes, Big Endian)
    pub fn read_word(&self, area: MemoryArea, db_num: u16, offset: usize) -> Option<u16> {
        self.read(area, db_num, offset, 2).map(|v| ((v[0] as u16) << 8) | (v[1] as u16))
    }
    
    /// Write word (2 bytes, Big Endian)
    pub fn write_word(&mut self, area: MemoryArea, db_num: u16, offset: usize, value: u16) -> bool {
        let bytes = [(value >> 8) as u8, (value & 0xFF) as u8];
        self.write(area, db_num, offset, &bytes)
    }
    
    /// Read dword (4 bytes, Big Endian)
    pub fn read_dword(&self, area: MemoryArea, db_num: u16, offset: usize) -> Option<u32> {
        self.read(area, db_num, offset, 4).map(|v| 
            ((v[0] as u32) << 24) | ((v[1] as u32) << 16) | 
            ((v[2] as u32) << 8) | (v[3] as u32)
        )
    }
    
    /// Write dword (4 bytes, Big Endian)
    pub fn write_dword(&mut self, area: MemoryArea, db_num: u16, offset: usize, value: u32) -> bool {
        let bytes = [
            (value >> 24) as u8,
            ((value >> 16) & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            (value & 0xFF) as u8
        ];
        self.write(area, db_num, offset, &bytes)
    }
    
    /// Read int16 (Big Endian, signed)
    pub fn read_int(&self, area: MemoryArea, db_num: u16, offset: usize) -> Option<i16> {
        self.read_word(area, db_num, offset).map(|v| i16::from_be_bytes([(v >> 8) as u8, (v & 0xFF) as u8]))
    }
    
    /// Write int16 (Big Endian, signed)
    pub fn write_int(&mut self, area: MemoryArea, db_num: u16, offset: usize, value: i16) -> bool {
        self.write_word(area, db_num, offset, value as u16)
    }
    
    /// Read float32 (Big Endian)
    pub fn read_real(&self, area: MemoryArea, db_num: u16, offset: usize) -> Option<f32> {
        self.read(area, db_num, offset, 4).map(|v| f32::from_be_bytes([v[0], v[1], v[2], v[3]]))
    }
    
    /// Write float32 (Big Endian)
    pub fn write_real(&mut self, area: MemoryArea, db_num: u16, offset: usize, value: f32) -> bool {
        let bytes = value.to_be_bytes();
        self.write(area, db_num, offset, &bytes)
    }
    
    /// Read string from DB
    pub fn read_string(&self, db_num: u16, offset: usize, max_len: usize) -> Option<String> {
        let data = self.read(MemoryArea::DataBlocks, db_num, offset, max_len + 4)?;
        if data.len() < 4 {
            return None;
        }
        let actual_len = data[3] as usize;
        if actual_len > max_len || 4 + actual_len > data.len() {
            return None;
        }
        String::from_utf8(data[4..4 + actual_len].to_vec()).ok()
    }
    
    /// Write string to DB (DB only, offset includes string header)
    pub fn write_string(&mut self, db_num: u16, offset: usize, value: &str) -> bool {
        let bytes = value.as_bytes();
        if bytes.len() > 254 {
            return false;
        }
        let mut data = vec![0u8; bytes.len() + 4];
        data[0] = 0xFF;
        data[1] = 0xFE;
        data[2] = 0x00;
        data[3] = bytes.len() as u8;
        data[4..].copy_from_slice(bytes);
        
        if let Some(db) = self.data_blocks.get_mut(&db_num) {
            if offset + data.len() <= db.bytes.len() {
                db.bytes[offset..offset + data.len()].copy_from_slice(&data);
                return true;
            }
        }
        false
    }
    
    /// Get DB size
    pub fn get_db_size(&self, db_num: u16) -> Option<usize> {
        self.data_blocks.get(&db_num).map(|db| db.bytes.len())
    }
    
    /// Get data block data (for API access)
    pub fn get_db_data(&self, db_num: u16) -> Option<&DataBlockData> {
        self.data_blocks.get(&db_num)
    }
    
    /// Get inputs as bytes
    pub fn get_inputs(&self) -> Vec<u8> {
        self.inputs.clone()
    }
    
    /// Get outputs as bytes
    pub fn get_outputs(&self) -> Vec<u8> {
        self.outputs.clone()
    }
    
    /// Get flags as bytes
    pub fn get_flags(&self) -> Vec<u8> {
        self.flags.clone()
    }
    
    /// Clear all data blocks
    pub fn clear_dbs(&mut self) {
        for db in self.data_blocks.values_mut() {
            db.bytes.fill(0);
        }
    }
    
    /// Clear specific data block
    pub fn clear_db(&mut self, db_num: u16) -> bool {
        if let Some(db) = self.data_blocks.get_mut(&db_num) {
            db.bytes.fill(0);
            true
        } else {
            false
        }
    }
}

/// Shared PLC Memory
pub type SharedMemory = Arc<RwLock<PlcMemory>>;

/// Create shared memory
pub fn create_shared_memory() -> SharedMemory {
    let mut memory = PlcMemory::new();
    memory.init_default_db();
    Arc::new(RwLock::new(memory))
}

/// Create shared memory from config file
pub fn create_shared_memory_from_config(path: &Path) -> Result<SharedMemory, String> {
    let memory = PlcMemory::from_config_file(path)?;
    Ok(Arc::new(RwLock::new(memory)))
}
