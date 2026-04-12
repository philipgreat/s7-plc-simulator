//!
//! PLC Memory Management
//! 
//! Manages memory areas and data blocks for the simulated PLC

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

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
pub struct DataType {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub data_type: String, // "BOOL", "BYTE", "WORD", "DWORD", "INT", "DINT", "REAL", "STRING"
}

/// Data block definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlock {
    pub number: u16,
    pub size: usize,
    pub variables: Vec<DataType>,
}

/// Data block with raw bytes
#[derive(Debug, Clone)]
pub struct DataBlockData {
    pub number: u16,
    pub bytes: Vec<u8>,
}

impl DataBlockData {
    pub fn new(number: u16, size: usize) -> Self {
        Self {
            number,
            bytes: vec![0u8; size],
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
    
    /// Initialize default data blocks
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
    }
    
    /// Add a new data block
    pub fn add_db(&mut self, number: u16, size: usize) {
        self.data_blocks.insert(number, DataBlockData::new(number, size));
    }
    
    /// Remove a data block
    pub fn remove_db(&mut self, number: u16) -> bool {
        self.data_blocks.remove(&number).is_some()
    }
    
    /// Get data block info
    pub fn get_db_info(&self, number: u16) -> Option<DataBlock> {
        self.data_blocks.get(&number).map(|db| DataBlock {
            number: db.number,
            size: db.bytes.len(),
            variables: Vec::new(),
        })
    }
    
    /// List all data blocks
    pub fn list_dbs(&self) -> Vec<DataBlock> {
        self.data_blocks.values().map(|db| DataBlock {
            number: db.number,
            size: db.bytes.len(),
            variables: Vec::new(),
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
