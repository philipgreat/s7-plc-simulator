//! Comprehensive unit tests for s7-plc-simulator
//!
//! Covers:
//! - MemoryArea enum conversions
//! - PlcMemory: read/write operations for all areas and data types
//! - DataBlockData: variable initialization and value retrieval
//! - PlcConfig: JSON loading
//! - S7 Protocol: Setup Communication, Read Var, Write Var request/response
//! - Connection tracking
//! - Edge cases: boundary checks, invalid inputs, overflow

use crate::*;

// ==================== MemoryArea Tests ====================

#[test]
fn test_memory_area_from_byte_valid() {
    assert_eq!(MemoryArea::from_byte(0x81), Some(MemoryArea::Inputs));
    assert_eq!(MemoryArea::from_byte(0x82), Some(MemoryArea::Outputs));
    assert_eq!(MemoryArea::from_byte(0x83), Some(MemoryArea::Flags));
    assert_eq!(MemoryArea::from_byte(0x84), Some(MemoryArea::DataBlocks));
    assert_eq!(MemoryArea::from_byte(0x1C), Some(MemoryArea::Counters));
    assert_eq!(MemoryArea::from_byte(0x1D), Some(MemoryArea::Timers));
}

#[test]
fn test_memory_area_from_byte_invalid() {
    assert_eq!(MemoryArea::from_byte(0x00), None);
    assert_eq!(MemoryArea::from_byte(0x80), None);
    assert_eq!(MemoryArea::from_byte(0x85), None);
    assert_eq!(MemoryArea::from_byte(0xFF), None);
}

#[test]
fn test_memory_area_byte_roundtrip() {
    for byte in [0x81u8, 0x82, 0x83, 0x84, 0x1C, 0x1D] {
        let area = MemoryArea::from_byte(byte).unwrap();
        assert_eq!(area as u8, byte);
    }
}

#[test]
fn test_memory_area_name() {
    assert_eq!(MemoryArea::Inputs.name(), "Inputs");
    assert_eq!(MemoryArea::Outputs.name(), "Outputs");
    assert_eq!(MemoryArea::Flags.name(), "Flags");
    assert_eq!(MemoryArea::DataBlocks.name(), "DataBlocks");
    assert_eq!(MemoryArea::Counters.name(), "Counters");
    assert_eq!(MemoryArea::Timers.name(), "Timers");
}

// ==================== PlcMemory Basic Tests ====================

fn create_test_memory() -> PlcMemory {
    let mut mem = PlcMemory::new();
    mem.init_default_db();
    mem
}

#[test]
fn test_plc_memory_new() {
    let mem = PlcMemory::new();
    assert_eq!(mem.db_count(), 0);
}

#[test]
fn test_add_db() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert_eq!(mem.db_count(), 1);
    mem.add_db(2, 128);
    assert_eq!(mem.db_count(), 2);
}

#[test]
fn test_add_db_overwrite() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    mem.add_db(1, 512); // Overwrite DB1 with different size
    assert_eq!(mem.db_count(), 1);
    assert_eq!(mem.get_db_size(1), Some(512));
}

#[test]
fn test_remove_db() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.remove_db(1));
    assert!(!mem.remove_db(1)); // Already removed
    assert!(!mem.remove_db(999)); // Never existed
}

#[test]
fn test_get_db_size() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert_eq!(mem.get_db_size(1), Some(256));
    assert_eq!(mem.get_db_size(999), None);
}

#[test]
fn test_list_dbs() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    mem.add_db(2, 128);
    mem.add_db(3, 64);
    let dbs = mem.list_dbs();
    assert_eq!(dbs.len(), 3);
}

#[test]
fn test_init_default_db() {
    let mem = create_test_memory();
    // Should have DB1, DB2, DB3, DB10, DB11, DB20, DB100, DB401, DB2991
    assert!(mem.get_db_size(1).is_some());
    assert!(mem.get_db_size(10).is_some());
    assert!(mem.get_db_size(11).is_some());
    assert!(mem.get_db_size(20).is_some());
    assert!(mem.get_db_size(100).is_some());
    assert!(mem.get_db_size(401).is_some());
    assert!(mem.get_db_size(2991).is_some());
}

// ==================== Read/Write Operations ====================

#[test]
fn test_read_write_db_bytes() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);

    // Write and read back
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    assert!(mem.write(MemoryArea::DataBlocks, 1, 0, &data));
    
    let result = mem.read(MemoryArea::DataBlocks, 1, 0, 4).unwrap();
    assert_eq!(result, data);
}

#[test]
fn test_read_db_nonexistent() {
    let mem = PlcMemory::new();
    assert!(mem.read(MemoryArea::DataBlocks, 999, 0, 4).is_none());
}

#[test]
fn test_write_db_nonexistent() {
    let mut mem = PlcMemory::new();
    assert!(!mem.write(MemoryArea::DataBlocks, 999, 0, &[0x01]));
}

#[test]
fn test_read_db_out_of_bounds() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 10);
    assert!(mem.read(MemoryArea::DataBlocks, 1, 0, 10).is_some());
    assert!(mem.read(MemoryArea::DataBlocks, 1, 0, 11).is_none()); // exceeds size
    assert!(mem.read(MemoryArea::DataBlocks, 1, 5, 6).is_none());  // start+len > size
}

#[test]
fn test_write_db_out_of_bounds() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 10);
    assert!(mem.write(MemoryArea::DataBlocks, 1, 0, &[0x01; 10]));
    assert!(!mem.write(MemoryArea::DataBlocks, 1, 0, &[0x01; 11])); // exceeds size
    assert!(!mem.write(MemoryArea::DataBlocks, 1, 8, &[0x01; 4]));  // start+len > size
}

#[test]
fn test_read_write_inputs() {
    let mut mem = PlcMemory::new();
    let data = vec![0x01, 0x02, 0x03];
    assert!(mem.write(MemoryArea::Inputs, 0, 0, &data));
    let result = mem.read(MemoryArea::Inputs, 0, 0, 3).unwrap();
    assert_eq!(result, data);
}

#[test]
fn test_read_write_outputs() {
    let mut mem = PlcMemory::new();
    let data = vec![0xAA, 0xBB];
    assert!(mem.write(MemoryArea::Outputs, 0, 0, &data));
    let result = mem.read(MemoryArea::Outputs, 0, 0, 2).unwrap();
    assert_eq!(result, data);
}

#[test]
fn test_read_write_flags() {
    let mut mem = PlcMemory::new();
    let data = vec![0xFF];
    assert!(mem.write(MemoryArea::Flags, 0, 0, &data));
    let result = mem.read(MemoryArea::Flags, 0, 0, 1).unwrap();
    assert_eq!(result, data);
}

#[test]
fn test_read_write_counters_timers_unsupported() {
    let mut mem = PlcMemory::new();
    assert!(mem.read(MemoryArea::Counters, 0, 0, 2).is_none());
    assert!(mem.read(MemoryArea::Timers, 0, 0, 4).is_none());
    assert!(!mem.write(MemoryArea::Counters, 0, 0, &[0x01]));
    assert!(!mem.write(MemoryArea::Timers, 0, 0, &[0x01]));
}

// ==================== Typed Read/Write ====================

#[test]
fn test_read_write_byte() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_byte(MemoryArea::DataBlocks, 1, 0, 0xAB));
    assert_eq!(mem.read_byte(MemoryArea::DataBlocks, 1, 0), Some(0xAB));
}

#[test]
fn test_read_write_word() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_word(MemoryArea::DataBlocks, 1, 0, 0x1234));
    assert_eq!(mem.read_word(MemoryArea::DataBlocks, 1, 0), Some(0x1234));
}

#[test]
fn test_read_write_dword() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_dword(MemoryArea::DataBlocks, 1, 0, 0xDEADBEEF));
    assert_eq!(mem.read_dword(MemoryArea::DataBlocks, 1, 0), Some(0xDEADBEEF));
}

#[test]
fn test_read_write_int_positive() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_int(MemoryArea::DataBlocks, 1, 0, 1000));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 1, 0), Some(1000));
}

#[test]
fn test_read_write_int_negative() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_int(MemoryArea::DataBlocks, 1, 0, -200));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 1, 0), Some(-200));
}

#[test]
fn test_read_write_int_boundary() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    // i16 min/max
    assert!(mem.write_int(MemoryArea::DataBlocks, 1, 0, i16::MAX));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 1, 0), Some(i16::MAX));
    assert!(mem.write_int(MemoryArea::DataBlocks, 1, 2, i16::MIN));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 1, 2), Some(i16::MIN));
}

#[test]
fn test_read_write_real() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    assert!(mem.write_real(MemoryArea::DataBlocks, 1, 0, 3.14));
    let result = mem.read_real(MemoryArea::DataBlocks, 1, 0).unwrap();
    assert!((result - 3.14f32).abs() < 0.001);
}

#[test]
fn test_read_write_real_special_values() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    
    assert!(mem.write_real(MemoryArea::DataBlocks, 1, 0, 0.0));
    assert_eq!(mem.read_real(MemoryArea::DataBlocks, 1, 0), Some(0.0f32));
    
    assert!(mem.write_real(MemoryArea::DataBlocks, 1, 4, f32::INFINITY));
    assert!(mem.read_real(MemoryArea::DataBlocks, 1, 4).unwrap().is_infinite());
    
    assert!(mem.write_real(MemoryArea::DataBlocks, 1, 8, f32::NEG_INFINITY));
    let neg_inf = mem.read_real(MemoryArea::DataBlocks, 1, 8).unwrap();
    assert!(neg_inf.is_infinite() && neg_inf.is_sign_negative());
}

#[test]
fn test_read_write_string() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 512);
    assert!(mem.write_string(1, 0, "Hello"));
    assert_eq!(mem.read_string(1, 0, 254), Some("Hello".to_string()));
}

#[test]
fn test_read_write_string_empty() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 512);
    assert!(mem.write_string(1, 0, ""));
    assert_eq!(mem.read_string(1, 0, 254), Some("".to_string()));
}

#[test]
fn test_read_write_string_too_long() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 512);
    let long_str = "A".repeat(255);
    assert!(!mem.write_string(1, 0, &long_str)); // >254 chars
}

#[test]
fn test_read_string_invalid_db() {
    let mem = PlcMemory::new();
    assert!(mem.read_string(999, 0, 254).is_none());
}

// ==================== Clear Operations ====================

#[test]
fn test_clear_db() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    mem.write_dword(MemoryArea::DataBlocks, 1, 0, 0xDEADBEEF);
    assert!(mem.clear_db(1));
    assert_eq!(mem.read_dword(MemoryArea::DataBlocks, 1, 0), Some(0));
}

#[test]
fn test_clear_db_nonexistent() {
    let mut mem = PlcMemory::new();
    assert!(!mem.clear_db(999));
}

#[test]
fn test_clear_all_dbs() {
    let mut mem = create_test_memory();
    mem.clear_dbs();
    // All DBs should be zeroed
    assert_eq!(mem.read_dword(MemoryArea::DataBlocks, 401, 0), Some(0));
}

// ==================== DataBlockData Variable Tests ====================

#[test]
fn test_datablock_new() {
    let db = DataBlockData::new(1, 256);
    assert_eq!(db.number, 1);
    assert_eq!(db.bytes.len(), 256);
    assert!(db.bytes.iter().all(|&b| b == 0));
}

#[test]
fn test_datablock_with_config_bool() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "flag".to_string(),
            offset: 0,
            data_type: "BOOL".to_string(),
            value: serde_json::json!(true),
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    assert_eq!(db.bytes[0], 0x01);
}

#[test]
fn test_datablock_with_config_byte() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "value".to_string(),
            offset: 0,
            data_type: "BYTE".to_string(),
            value: serde_json::json!(200),
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    assert_eq!(db.bytes[0], 200);
}

#[test]
fn test_datablock_with_config_word() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "counter".to_string(),
            offset: 0,
            data_type: "WORD".to_string(),
            value: serde_json::json!(4660), // 0x1234
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    assert_eq!(db.bytes[0], 0x12);
    assert_eq!(db.bytes[1], 0x34);
}

#[test]
fn test_datablock_with_config_dword() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "large".to_string(),
            offset: 0,
            data_type: "DWORD".to_string(),
            value: serde_json::json!(305419896), // 0x12345678
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    assert_eq!(db.bytes[0], 0x12);
    assert_eq!(db.bytes[1], 0x34);
    assert_eq!(db.bytes[2], 0x56);
    assert_eq!(db.bytes[3], 0x78);
}

#[test]
fn test_datablock_with_config_int() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "signed_val".to_string(),
            offset: 0,
            data_type: "INT".to_string(),
            value: serde_json::json!(-1000),
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    let val = i16::from_be_bytes([db.bytes[0], db.bytes[1]]);
    assert_eq!(val, -1000);
}

#[test]
fn test_datablock_with_config_dint() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "big_signed".to_string(),
            offset: 0,
            data_type: "DINT".to_string(),
            value: serde_json::json!(-100000),
            unit: None,
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    let val = i32::from_be_bytes([db.bytes[0], db.bytes[1], db.bytes[2], db.bytes[3]]);
    assert_eq!(val, -100000);
}

#[test]
fn test_datablock_with_config_real() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: None,
        variables: vec![VariableDefinition {
            name: "temperature".to_string(),
            offset: 0,
            data_type: "REAL".to_string(),
            value: serde_json::json!(22.5),
            unit: Some("°C".to_string()),
            description: None,
            max_length: None,
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    let val = f32::from_be_bytes([db.bytes[0], db.bytes[1], db.bytes[2], db.bytes[3]]);
    assert!((val - 22.5f32).abs() < 0.001);
}

#[test]
fn test_datablock_with_config_string() {
    let config = DataBlockConfig {
        number: 1,
        size: 512,
        description: None,
        variables: vec![VariableDefinition {
            name: "message".to_string(),
            offset: 0,
            data_type: "STRING".to_string(),
            value: serde_json::json!("Hello"),
            unit: None,
            description: None,
            max_length: Some(254),
            enum_values: None,
        }],
    };
    let db = DataBlockData::with_config(&config);
    // S7 string format: [max_hi][max_lo][len_hi][len_lo][data...]
    assert_eq!(db.bytes[0], 0xFF); // max length hi
    assert_eq!(db.bytes[1], 0xFE); // max length lo
    assert_eq!(db.bytes[2], 0x00); // actual length hi
    assert_eq!(db.bytes[3], 0x05); // actual length lo
    assert_eq!(&db.bytes[4..9], b"Hello");
}

#[test]
fn test_datablock_with_config_multiple_variables() {
    let config = DataBlockConfig {
        number: 1,
        size: 256,
        description: Some("Test DB".to_string()),
        variables: vec![
            VariableDefinition {
                name: "status".to_string(),
                offset: 0,
                data_type: "INT".to_string(),
                value: serde_json::json!(1),
                unit: None,
                description: None,
                max_length: None,
                enum_values: None,
            },
            VariableDefinition {
                name: "temperature".to_string(),
                offset: 2,
                data_type: "REAL".to_string(),
                value: serde_json::json!(25.0),
                unit: Some("°C".to_string()),
                description: None,
                max_length: None,
                enum_values: None,
            },
        ],
    };
    let db = DataBlockData::with_config(&config);
    assert_eq!(db.description, Some("Test DB".to_string()));
    let status = i16::from_be_bytes([db.bytes[0], db.bytes[1]]);
    assert_eq!(status, 1);
    let temp = f32::from_be_bytes([db.bytes[2], db.bytes[3], db.bytes[4], db.bytes[5]]);
    assert!((temp - 25.0f32).abs() < 0.001);
}

// ==================== get_variable_value Tests ====================

#[test]
fn test_get_variable_value_bool() {
    let mut db = DataBlockData::new(1, 256);
    db.bytes[0] = 0x01;
    let var = VariableDefinition {
        name: "flag".to_string(),
        offset: 0,
        data_type: "BOOL".to_string(),
        value: serde_json::json!(true),
        unit: None,
        description: None,
        max_length: None,
        enum_values: None,
    };
    assert_eq!(db.get_variable_value(&var), serde_json::json!(true));
}

#[test]
fn test_get_variable_value_int() {
    let mut db = DataBlockData::new(1, 256);
    let val: i16 = -500;
    db.bytes[0..2].copy_from_slice(&val.to_be_bytes());
    let var = VariableDefinition {
        name: "value".to_string(),
        offset: 0,
        data_type: "INT".to_string(),
        value: serde_json::json!(-500),
        unit: None,
        description: None,
        max_length: None,
        enum_values: None,
    };
    assert_eq!(db.get_variable_value(&var), serde_json::json!(-500));
}

#[test]
fn test_get_variable_value_real() {
    let mut db = DataBlockData::new(1, 256);
    let val: f32 = 3.14;
    db.bytes[0..4].copy_from_slice(&val.to_be_bytes());
    let var = VariableDefinition {
        name: "pi".to_string(),
        offset: 0,
        data_type: "REAL".to_string(),
        value: serde_json::json!(3.14),
        unit: None,
        description: None,
        max_length: None,
        enum_values: None,
    };
    let result = db.get_variable_value(&var);
    assert!((result.as_f64().unwrap() - 3.14f64).abs() < 0.01);
}

#[test]
fn test_get_variable_value_out_of_bounds() {
    let db = DataBlockData::new(1, 2);
    let var = VariableDefinition {
        name: "overflow".to_string(),
        offset: 0,
        data_type: "DWORD".to_string(), // needs 4 bytes, only 2 available
        value: serde_json::json!(0),
        unit: None,
        description: None,
        max_length: None,
        enum_values: None,
    };
    assert_eq!(db.get_variable_value(&var), serde_json::json!(null));
}

#[test]
fn test_get_variable_value_string() {
    let mut db = DataBlockData::new(1, 256);
    // S7 string: [FF FE 00 05] + "Hello"
    db.bytes[0] = 0xFF;
    db.bytes[1] = 0xFE;
    db.bytes[2] = 0x00;
    db.bytes[3] = 0x05;
    db.bytes[4..9].copy_from_slice(b"Hello");
    let var = VariableDefinition {
        name: "msg".to_string(),
        offset: 0,
        data_type: "STRING".to_string(),
        value: serde_json::json!("Hello"),
        unit: None,
        description: None,
        max_length: None,
        enum_values: None,
    };
    assert_eq!(db.get_variable_value(&var), serde_json::json!("Hello"));
}

// ==================== Default DB Initialization Tests ====================

#[test]
fn test_default_db10_reals() {
    let mem = create_test_memory();
    // DB10 has 4 REALs: 1.5, 2.5, 3.14, 100.0
    let val0 = mem.read_real(MemoryArea::DataBlocks, 10, 0).unwrap();
    assert!((val0 - 1.5f32).abs() < 0.001);
    let val4 = mem.read_real(MemoryArea::DataBlocks, 10, 4).unwrap();
    assert!((val4 - 2.5f32).abs() < 0.001);
    let val8 = mem.read_real(MemoryArea::DataBlocks, 10, 8).unwrap();
    assert!((val8 - 3.14f32).abs() < 0.01);
    let val12 = mem.read_real(MemoryArea::DataBlocks, 10, 12).unwrap();
    assert!((val12 - 100.0f32).abs() < 0.001);
}

#[test]
fn test_default_db11_ints() {
    let mem = create_test_memory();
    // DB11 has 8 INTs: 100, -200, 300, -400, 500, -600, 700, -800
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 11, 0), Some(100));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 11, 2), Some(-200));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 11, 4), Some(300));
    assert_eq!(mem.read_int(MemoryArea::DataBlocks, 11, 6), Some(-400));
}

#[test]
fn test_default_db20_string() {
    let mem = create_test_memory();
    // DB20 is 128 bytes, read_string reads max_len+4 bytes, so max_len must be <= 124
    let s = mem.read_string(20, 0, 124).unwrap();
    assert_eq!(s, "Hello World!");
}

#[test]
fn test_default_db100_status() {
    let mem = create_test_memory();
    // status at offset 0: DWORD = 1 (RUNNING)
    let status = mem.read_dword(MemoryArea::DataBlocks, 100, 0).unwrap();
    assert_eq!(status, 1);
    // taskId at offset 4: DWORD = 12345
    let task_id = mem.read_dword(MemoryArea::DataBlocks, 100, 4).unwrap();
    assert_eq!(task_id, 12345);
    // progress at offset 8: REAL = 65.5
    let progress = mem.read_real(MemoryArea::DataBlocks, 100, 8).unwrap();
    assert!((progress - 65.5f32).abs() < 0.1);
}

#[test]
fn test_default_db401() {
    let mem = create_test_memory();
    // stationStatus at offset 0: DWORD = 1
    let status = mem.read_dword(MemoryArea::DataBlocks, 401, 0).unwrap();
    assert_eq!(status, 1);
    // currentTaskId at offset 4: DWORD = 67890
    let task_id = mem.read_dword(MemoryArea::DataBlocks, 401, 4).unwrap();
    assert_eq!(task_id, 67890);
    // fillProgress at offset 8: REAL = 72.5
    let progress = mem.read_real(MemoryArea::DataBlocks, 401, 8).unwrap();
    assert!((progress - 72.5f32).abs() < 0.1);
}

// ==================== SharedMemory Tests ====================

#[test]
fn test_create_shared_memory() {
    let shared = create_shared_memory();
    let mem = shared.read().unwrap();
    assert!(mem.db_count() > 0);
}

#[test]
fn test_shared_memory_concurrent_read() {
    let shared = create_shared_memory();
    let shared2 = shared.clone();
    
    let mem1 = shared.read().unwrap();
    let mem2 = shared2.read().unwrap();
    // Multiple readers should work
    assert_eq!(mem1.db_count(), mem2.db_count());
}

#[test]
fn test_shared_memory_write_then_read() {
    let shared = create_shared_memory();
    {
        let mut mem = shared.write().unwrap();
        mem.write_word(MemoryArea::DataBlocks, 1, 0, 0xBEEF);
    }
    let mem = shared.read().unwrap();
    assert_eq!(mem.read_word(MemoryArea::DataBlocks, 1, 0), Some(0xBEEF));
}

// ==================== Connection Tracking Tests ====================

#[test]
fn test_connection_list_new() {
    let list = create_connection_list();
    let conns = list.read().unwrap();
    assert_eq!(conns.len(), 0);
}

#[test]
fn test_connection_list_add_remove() {
    let list = create_connection_list();
    
    let conn = ClientConnection {
        id: 1,
        remote_addr: "127.0.0.1:12345".to_string(),
        connected_at: "2026-01-01T00:00:00Z".to_string(),
        last_activity: "2026-01-01T00:00:00Z".to_string(),
        requests_count: 0,
        framing: "TPKT".to_string(),
        state: "connected".to_string(),
    };
    
    {
        let mut conns = list.write().unwrap();
        conns.push(conn);
    }
    
    {
        let conns = list.read().unwrap();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].id, 1);
        assert_eq!(conns[0].framing, "TPKT");
    }
    
    {
        let mut conns = list.write().unwrap();
        conns.retain(|c| c.id != 1);
    }
    
    {
        let conns = list.read().unwrap();
        assert_eq!(conns.len(), 0);
    }
}

#[test]
fn test_connection_list_multiple() {
    let list = create_connection_list();
    
    let conns_to_add = vec![
        ClientConnection {
            id: 1,
            remote_addr: "10.0.0.1:11111".to_string(),
            connected_at: "2026-01-01T00:00:00Z".to_string(),
            last_activity: "2026-01-01T00:00:00Z".to_string(),
            requests_count: 5,
            framing: "TPKT".to_string(),
            state: "connected".to_string(),
        },
        ClientConnection {
            id: 2,
            remote_addr: "10.0.0.2:22222".to_string(),
            connected_at: "2026-01-01T00:01:00Z".to_string(),
            last_activity: "2026-01-01T00:01:00Z".to_string(),
            requests_count: 3,
            framing: "Raw COTP".to_string(),
            state: "connected".to_string(),
        },
    ];
    
    {
        let mut conns = list.write().unwrap();
        conns.extend(conns_to_add);
    }
    
    {
        let conns = list.read().unwrap();
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[0].requests_count, 5);
        assert_eq!(conns[1].framing, "Raw COTP");
    }
}

#[test]
fn test_client_connection_serialize() {
    let conn = ClientConnection {
        id: 42,
        remote_addr: "192.168.1.100:54321".to_string(),
        connected_at: "2026-04-26T00:00:00Z".to_string(),
        last_activity: "2026-04-26T00:05:00Z".to_string(),
        requests_count: 100,
        framing: "TPKT".to_string(),
        state: "connected".to_string(),
    };
    let json = serde_json::to_string(&conn).unwrap();
    assert!(json.contains("\"id\":42"));
    assert!(json.contains("192.168.1.100"));
}

// ==================== S7 Protocol Response Building Tests ====================

/// Helper to create a PlcSimulator for testing
fn create_test_simulator() -> PlcSimulator {
    let memory = create_shared_memory();
    let connections = create_connection_list();
    let log_buffer = create_log_buffer();
    PlcSimulator::new("S7-1200", 0, 1, memory, connections, log_buffer)
}

#[test]
fn test_build_setup_response() {
    let sim = create_test_simulator();
    let resp = sim.build_setup_response(0x0001);
    
    // S7 header
    assert_eq!(resp[0], 0x32); // Protocol ID
    assert_eq!(resp[1], 0x03); // AckData
    assert_eq!(resp[2], 0x00); // Reserved
    assert_eq!(resp[3], 0x00); // Reserved
    assert_eq!(resp[4], 0x00); // Ref high
    assert_eq!(resp[5], 0x01); // Ref low
    
    // Param len = 8
    assert_eq!(((resp[6] as u16) << 8) | resp[7] as u16, 8);
    // Data len = 0
    assert_eq!(((resp[8] as u16) << 8) | resp[9] as u16, 0);
    
    // Param data: function code 0xF0 + 0x00 + max AmQ + max AmQ + PDU size
    assert_eq!(resp[10], 0xF0); // Setup Communication function code
    assert_eq!(resp[11], 0x00); // Reserved
    
    // PDU size = 480 (0x01E0)
    assert_eq!(resp[16], 0x01);
    assert_eq!(resp[17], 0xE0);
}

#[test]
fn test_build_setup_response_different_refs() {
    let sim = create_test_simulator();
    let resp1 = sim.build_setup_response(0x0001);
    let resp2 = sim.build_setup_response(0x1234);
    
    assert_eq!(resp1[4], 0x00);
    assert_eq!(resp1[5], 0x01);
    assert_eq!(resp2[4], 0x12);
    assert_eq!(resp2[5], 0x34);
}

#[test]
fn test_build_user_data_response() {
    let sim = create_test_simulator();
    let resp = sim.build_user_data_response(0x0005);
    
    assert_eq!(resp[0], 0x32); // Protocol ID
    assert_eq!(resp[1], 0x03); // AckData
    // Ref
    assert_eq!(((resp[4] as u16) << 8) | resp[5] as u16, 5);
    // Param len = 4
    assert_eq!(((resp[6] as u16) << 8) | resp[7] as u16, 4);
    // Data len = 0
    assert_eq!(((resp[8] as u16) << 8) | resp[9] as u16, 0);
}

// ==================== S7 Read Var Request/Response Tests ====================

/// Build a standard S7 Read Var request param_data
/// Read `num_elements` bytes from DB `db_num` at byte offset `start_offset`
fn build_read_var_param(item_count: u8, items: &[(u16, u32, u16)]) -> Vec<u8> {
    let mut param = vec![0x04, item_count]; // Function code + item count
    for &(db_num, start_offset, num_elements) in items {
        // Item spec: [0x12] [spec_len=0x0A] [0x10] [transport=0x02] [num_elements:2] [db_num:2] [area=0x84] [offset:3]
        param.push(0x12); // Spec type
        param.push(0x0A); // Spec len = 10
        param.push(0x10); // Syntax ID
        param.push(0x02); // Transport size = BYTE
        param.extend_from_slice(&num_elements.to_be_bytes());
        param.extend_from_slice(&db_num.to_be_bytes());
        param.push(0x84); // Area = DataBlocks
        // Offset: 3 bytes
        param.push(((start_offset >> 16) & 0xFF) as u8);
        param.push(((start_offset >> 8) & 0xFF) as u8);
        param.push((start_offset & 0xFF) as u8);
    }
    param
}

#[tokio::test]
async fn test_handle_read_request_db401() {
    let sim = create_test_simulator();
    let param = build_read_var_param(1, &[(401, 0, 38)]);
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    // Verify response header
    assert_eq!(resp[0], 0x32); // Protocol ID
    assert_eq!(resp[1], 0x03); // AckData
    // Ref
    assert_eq!(((resp[4] as u16) << 8) | resp[5] as u16, 1);
    
    // Param section: [0x04, item_count=1]
    assert_eq!(resp[10], 0x04); // Read function code
    assert_eq!(resp[11], 0x01); // Item count
    
    // Data section: [return_code, transport_size, len_hi, len_lo, data...]
    let data_start = 12;
    assert_eq!(resp[data_start], 0xFF); // Success return code
    assert_eq!(resp[data_start + 1], 0x02); // Transport size = BYTE
    let data_len = ((resp[data_start + 2] as u16) << 8) | resp[data_start + 3] as u16;
    assert_eq!(data_len, 38);
    
    // Verify DB401 first 4 bytes = stationStatus = 1
    assert_eq!(resp[data_start + 4], 0x00);
    assert_eq!(resp[data_start + 5], 0x00);
    assert_eq!(resp[data_start + 6], 0x00);
    assert_eq!(resp[data_start + 7], 0x01);
}

#[tokio::test]
async fn test_handle_read_request_nonexistent_db() {
    let sim = create_test_simulator();
    let param = build_read_var_param(1, &[(999, 0, 4)]);
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    // Data section should have error return code
    let data_start = 12;
    assert_eq!(resp[data_start], 0x0A); // Error: data not available
}

#[tokio::test]
async fn test_handle_read_request_out_of_bounds() {
    let sim = create_test_simulator();
    // DB401 is 38 bytes, read from offset 36 with len 4 -> out of bounds
    let param = build_read_var_param(1, &[(401, 36, 4)]);
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    let data_start = 12;
    assert_eq!(resp[data_start], 0x0A); // Error: data not available
}

#[tokio::test]
async fn test_handle_read_request_multiple_items() {
    let sim = create_test_simulator();
    // Read 4 bytes from DB401 offset 0, and 4 bytes from DB401 offset 4
    let param = build_read_var_param(2, &[(401, 0, 4), (401, 4, 4)]);
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    // Param: [0x04, 0x02]
    assert_eq!(resp[10], 0x04);
    assert_eq!(resp[11], 0x02);
    
    // First item data: success
    assert_eq!(resp[12], 0xFF);
    let len1 = ((resp[14] as u16) << 8) | resp[15] as u16;
    assert_eq!(len1, 4);
    
    // Second item starts after 4 + 4 header + 4 data = 8
    let item2_start = 12 + 4 + 4;
    assert_eq!(resp[item2_start], 0xFF);
}

#[tokio::test]
async fn test_handle_read_request_invalid_spec_type() {
    let sim = create_test_simulator();
    // Wrong spec type (0x11 instead of 0x12)
    let param = vec![0x04, 0x01, 0x11, 0x0A, 0x10, 0x02, 0x00, 0x04, 0x01, 0x91, 0x84, 0x00, 0x00, 0x00];
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    // Should return error
    let data_start = 12;
    assert_eq!(resp[data_start], 0x0A); // Error
}

#[tokio::test]
async fn test_handle_read_request_short_spec() {
    let sim = create_test_simulator();
    // spec_len = 8 (too short, minimum is 10)
    let param = vec![0x04, 0x01, 0x12, 0x08, 0x10, 0x02, 0x00, 0x04, 0x01, 0x91];
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    let data_start = 12;
    assert_eq!(resp[data_start], 0x0A); // Error
}

#[tokio::test]
async fn test_handle_read_request_empty_param() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_read_request(0x0001, &[]).await;
    assert!(resp.is_none());
}

#[tokio::test]
async fn test_handle_read_request_too_short() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_read_request(0x0001, &[0x04]).await;
    assert!(resp.is_none());
}

#[tokio::test]
async fn test_handle_read_request_inputs_area() {
    let sim = create_test_simulator();
    
    // Write some data to inputs first
    {
        let mut mem = sim.memory.write().unwrap();
        assert!(mem.write(MemoryArea::Inputs, 0, 0, &[0xAA, 0xBB]));
    }
    
    // Build Read Var for Inputs area (0x81)
    let mut param = vec![0x04, 0x01]; // Function code + item count
    param.push(0x12); // Spec type
    param.push(0x0A); // Spec len
    param.push(0x10); // Syntax ID
    param.push(0x02); // Transport size
    param.extend_from_slice(&2u16.to_be_bytes()); // num_elements
    param.extend_from_slice(&0u16.to_be_bytes()); // db_num (unused for inputs)
    param.push(0x81); // Area = Inputs
    param.push(0x00); // Offset byte 0
    param.push(0x00); // Offset byte 1
    param.push(0x00); // Offset byte 2
    
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    let data_start = 12;
    assert_eq!(resp[data_start], 0xFF); // Success
    let len = ((resp[data_start + 2] as u16) << 8) | resp[data_start + 3] as u16;
    assert_eq!(len, 2);
    assert_eq!(resp[data_start + 4], 0xAA);
    assert_eq!(resp[data_start + 5], 0xBB);
}

// ==================== S7 Write Var Request/Response Tests ====================

/// Build a standard S7 Write Var request param_data
/// Write `data` bytes to DB `db_num` at byte offset `start_offset`
fn build_write_var_param(db_num: u16, start_offset: u32, data: &[u8]) -> Vec<u8> {
    let num_elements = data.len() as u16;
    let mut param = vec![0x05, 0x01]; // Function code + item count
    // Item spec
    param.push(0x12); // Spec type
    param.push(0x0A); // Spec len = 10
    param.push(0x10); // Syntax ID
    param.push(0x02); // Transport size = BYTE
    param.extend_from_slice(&num_elements.to_be_bytes());
    param.extend_from_slice(&db_num.to_be_bytes());
    param.push(0x84); // Area = DataBlocks
    // Offset: 3 bytes
    param.push(((start_offset >> 16) & 0xFF) as u8);
    param.push(((start_offset >> 8) & 0xFF) as u8);
    param.push((start_offset & 0xFF) as u8);
    // Data follows spec
    param.extend_from_slice(data);
    param
}

#[tokio::test]
async fn test_handle_write_request_basic() {
    let sim = create_test_simulator();
    let param = build_write_var_param(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
    let resp = sim.handle_write_request(0x0001, &param).await.unwrap();
    
    // Response header
    assert_eq!(resp[0], 0x32); // Protocol ID
    assert_eq!(resp[1], 0x03); // AckData
    
    // Param: [0x05, item_count=1]
    assert_eq!(resp[10], 0x05); // Write function code
    assert_eq!(resp[11], 0x01); // Item count
    
    // Data section: one return code byte per item
    assert_eq!(resp[12], 0xFF); // Success
    
    // Verify data was actually written
    let mem = sim.memory.read().unwrap();
    let read_data = mem.read(MemoryArea::DataBlocks, 1, 0, 4).unwrap();
    assert_eq!(read_data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

#[tokio::test]
async fn test_handle_write_request_nonexistent_db() {
    let sim = create_test_simulator();
    let param = build_write_var_param(999, 0, &[0x01]);
    let resp = sim.handle_write_request(0x0001, &param).await.unwrap();
    
    // Data section: error return code
    assert_eq!(resp[12], 0x0A); // Error
}

#[tokio::test]
async fn test_handle_write_request_out_of_bounds() {
    let sim = create_test_simulator();
    // DB401 is 38 bytes, write at offset 36 with len 4 -> out of bounds
    let param = build_write_var_param(401, 36, &[0x01, 0x02, 0x03, 0x04]);
    let resp = sim.handle_write_request(0x0001, &param).await.unwrap();
    
    assert_eq!(resp[12], 0x0A); // Error
}

#[tokio::test]
async fn test_handle_write_request_then_read() {
    let sim = create_test_simulator();
    
    // Write
    let write_param = build_write_var_param(1, 0, &[0xCA, 0xFE]);
    sim.handle_write_request(0x0001, &write_param).await.unwrap();
    
    // Read back
    let read_param = build_read_var_param(1, &[(1, 0, 2)]);
    let read_resp = sim.handle_read_request(0x0002, &read_param).await.unwrap();
    
    let data_start = 12 + 4; // After header + param + data header
    assert_eq!(read_resp[data_start], 0xCA);
    assert_eq!(read_resp[data_start + 1], 0xFE);
}

#[tokio::test]
async fn test_handle_write_request_empty_param() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_write_request(0x0001, &[]).await;
    assert!(resp.is_none());
}

#[tokio::test]
async fn test_handle_write_request_invalid_spec() {
    let sim = create_test_simulator();
    let param = vec![0x05, 0x01, 0x11, 0x0A, 0x10, 0x02, 0x00, 0x01, 0x00, 0x01, 0x84, 0x00, 0x00, 0x00, 0xFF];
    let resp = sim.handle_write_request(0x0001, &param).await.unwrap();
    assert_eq!(resp[12], 0x0A); // Error
}

// ==================== S7 Offset 3-byte Parsing Tests ====================

#[tokio::test]
async fn test_read_var_offset_3byte_large_offset() {
    let sim = create_test_simulator();
    
    // Add a large DB and write data at a high offset
    {
        let mut mem = sim.memory.write().unwrap();
        mem.add_db(50, 65536);
        assert!(mem.write(MemoryArea::DataBlocks, 50, 0x0100, &[0xAA, 0xBB]));
    }
    
    // Build Read Var with 3-byte offset = 0x000100 (256)
    let mut param = vec![0x04, 0x01]; // Function + count
    param.push(0x12); // Spec type
    param.push(0x0A); // Spec len
    param.push(0x10); // Syntax ID
    param.push(0x02); // Transport size
    param.extend_from_slice(&2u16.to_be_bytes()); // num_elements
    param.extend_from_slice(&50u16.to_be_bytes()); // db_num
    param.push(0x84); // Area = DataBlocks
    param.push(0x00); // Offset byte 0
    param.push(0x01); // Offset byte 1
    param.push(0x00); // Offset byte 2
    
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    let data_start = 12;
    assert_eq!(resp[data_start], 0xFF); // Success
    assert_eq!(resp[data_start + 4], 0xAA);
    assert_eq!(resp[data_start + 5], 0xBB);
}

#[tokio::test]
async fn test_write_var_offset_3byte_large_offset() {
    let sim = create_test_simulator();
    
    // Add a large DB
    {
        let mut mem = sim.memory.write().unwrap();
        mem.add_db(50, 65536);
    }
    
    // Build Write Var with 3-byte offset = 0x000200 (512)
    let mut param = vec![0x05, 0x01];
    param.push(0x12);
    param.push(0x0A);
    param.push(0x10);
    param.push(0x02);
    param.extend_from_slice(&2u16.to_be_bytes());
    param.extend_from_slice(&50u16.to_be_bytes());
    param.push(0x84);
    param.push(0x00); // Offset byte 0
    param.push(0x02); // Offset byte 1
    param.push(0x00); // Offset byte 2
    param.extend_from_slice(&[0xCC, 0xDD]);
    
    let resp = sim.handle_write_request(0x0001, &param).await.unwrap();
    assert_eq!(resp[12], 0xFF); // Success
    
    // Verify
    let mem = sim.memory.read().unwrap();
    let data = mem.read(MemoryArea::DataBlocks, 50, 0x0200, 2).unwrap();
    assert_eq!(data, vec![0xCC, 0xDD]);
}

// ==================== PlcConfig JSON Tests ====================

#[test]
fn test_plc_config_parse_minimal() {
    let json = r#"{
        "plc": { "type": "S7-1200", "rack": 0, "slot": 1 },
        "memory": { "inputs": { "size": 256 }, "outputs": { "size": 256 }, "flags": { "size": 1024 } },
        "data_blocks": [
            { "number": 1, "size": 256 }
        ]
    }"#;
    
    let config: PlcConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.plc.plc_type, "S7-1200");
    assert_eq!(config.plc.rack, 0);
    assert_eq!(config.plc.slot, 1);
    assert_eq!(config.data_blocks.len(), 1);
    assert_eq!(config.data_blocks[0].number, 1);
}

#[test]
fn test_plc_config_parse_with_variables() {
    let json = r#"{
        "plc": { "type": "S7-1500", "rack": 0, "slot": 1 },
        "memory": { "inputs": { "size": 256 }, "outputs": { "size": 256 }, "flags": { "size": 1024 } },
        "data_blocks": [
            {
                "number": 100,
                "size": 256,
                "description": "Filling Station",
                "variables": [
                    { "name": "status", "offset": 0, "type": "INT", "value": 1 },
                    { "name": "temperature", "offset": 2, "type": "REAL", "value": 22.5, "unit": "°C" },
                    { "name": "mode", "offset": 6, "type": "BYTE", "value": 3, "enum_values": {"0": "STOP", "1": "RUN", "3": "AUTO"} }
                ]
            }
        ]
    }"#;
    
    let config: PlcConfig = serde_json::from_str(json).unwrap();
    let db = &config.data_blocks[0];
    assert_eq!(db.number, 100);
    assert_eq!(db.size, 256);
    assert_eq!(db.description, Some("Filling Station".to_string()));
    assert_eq!(db.variables.len(), 3);
    
    let temp_var = &db.variables[1];
    assert_eq!(temp_var.name, "temperature");
    assert_eq!(temp_var.offset, 2);
    assert_eq!(temp_var.data_type, "REAL");
    assert_eq!(temp_var.unit, Some("°C".to_string()));
    
    let mode_var = &db.variables[2];
    assert!(mode_var.enum_values.is_some());
    let enum_vals = mode_var.enum_values.as_ref().unwrap();
    assert_eq!(enum_vals.get("3"), Some(&"AUTO".to_string()));
}

#[test]
fn test_plc_config_from_file_nonexistent() {
    let result = PlcMemory::from_config_file(std::path::Path::new("/nonexistent/path/config.json"));
    assert!(result.is_err());
}

// ==================== PlcSimulator handle_request Tests ====================

#[tokio::test]
async fn test_handle_request_setup_communication() {
    let sim = create_test_simulator();
    let param = vec![0xF0, 0x00, 0x03, 0xE8, 0x03, 0xE8, 0x01, 0xE0];
    let resp = sim.handle_request(0x01, 0x0001, &param, 1).await;
    assert!(resp.is_some());
    let r = resp.unwrap();
    assert_eq!(r[1], 0x03); // AckData
    assert_eq!(r[10], 0xF0); // Setup Communication
}

#[tokio::test]
async fn test_handle_request_read() {
    let sim = create_test_simulator();
    let param = build_read_var_param(1, &[(1, 0, 4)]);
    let resp = sim.handle_request(0x01, 0x0001, &param, 1).await;
    assert!(resp.is_some());
}

#[tokio::test]
async fn test_handle_request_write() {
    let sim = create_test_simulator();
    let param = build_write_var_param(1, 0, &[0x01, 0x02]);
    let resp = sim.handle_request(0x01, 0x0001, &param, 1).await;
    assert!(resp.is_some());
}

#[tokio::test]
async fn test_handle_request_user_data() {
    let sim = create_test_simulator();
    let param = vec![0x00, 0x00, 0x00, 0x00];
    let resp: Option<Vec<u8>> = sim.handle_request(0x07, 0x0001, &param, 1).await;
    assert!(resp.is_some());
}

#[tokio::test]
async fn test_handle_request_unknown_pdu_type() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_request(0xFF, 0x0001, &[0x00], 1).await;
    assert!(resp.is_none());
}

#[tokio::test]
async fn test_handle_request_unknown_function() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_request(0x01, 0x0001, &[0xAA], 1).await;
    assert!(resp.is_none());
}

#[tokio::test]
async fn test_handle_request_empty_param() {
    let sim = create_test_simulator();
    let resp: Option<Vec<u8>> = sim.handle_request(0x01, 0x0001, &[], 1).await;
    assert!(resp.is_none());
}

// ==================== S7 Full Packet Construction Tests ====================

#[test]
fn test_s7_setup_request_construction() {
    // Verify a well-formed S7 Setup Communication request packet
    let packet: Vec<u8> = vec![
        0x32,       // Protocol ID
        0x01,       // Job
        0x00, 0x00, // Reserved
        0x00, 0x01, // Ref
        0x00, 0x08, // Param len = 8
        0x00, 0x00, // Data len = 0
        // Param
        0xF0,       // Setup Communication
        0x00,       // Reserved
        0x03, 0xE8, // Max AmQ calling
        0x03, 0xE8, // Max AmQ called
        0x01, 0xE0, // PDU size = 480
    ];
    
    assert_eq!(packet.len(), 18);
    assert_eq!(packet[0], 0x32);
    let param_len = ((packet[6] as u16) << 8) | packet[7] as u16;
    let data_len = ((packet[8] as u16) << 8) | packet[9] as u16;
    assert_eq!(param_len, 8);
    assert_eq!(data_len, 0);
    assert_eq!(packet.len(), 10 + param_len as usize + data_len as usize);
}

#[test]
fn test_s7_read_var_request_construction() {
    // Verify a Read Var request for DB401, offset 0, 38 bytes
    let param = build_read_var_param(1, &[(401, 0, 38)]);
    
    assert_eq!(param[0], 0x04); // Read function code
    assert_eq!(param[1], 0x01); // Item count
    
    // Item spec starts at offset 2
    assert_eq!(param[2], 0x12); // Spec type
    assert_eq!(param[3], 0x0A); // Spec len = 10
    
    // DB number
    let db_num = ((param[8] as u16) << 8) | param[9] as u16;
    assert_eq!(db_num, 401);
    
    // Area
    assert_eq!(param[10], 0x84); // DataBlocks
    
    // Offset (3 bytes)
    assert_eq!(param[11], 0x00);
    assert_eq!(param[12], 0x00);
    assert_eq!(param[13], 0x00);
    
    // Total param length: 2 (header) + 2 (spec_type + spec_len) + 10 (spec data) = 14
    assert_eq!(param.len(), 14);
}

#[test]
fn test_s7_write_var_request_construction() {
    let param = build_write_var_param(1, 0, &[0xAA, 0xBB]);
    
    assert_eq!(param[0], 0x05); // Write function code
    assert_eq!(param[1], 0x01); // Item count
    
    // Item spec
    assert_eq!(param[2], 0x12); // Spec type
    assert_eq!(param[3], 0x0A); // Spec len = 10
    
    // num_elements
    let num = ((param[6] as u16) << 8) | param[7] as u16;
    assert_eq!(num, 2);
    
    // Data after spec
    let data_start = 2 + 2 + 10; // header + spec_type/len + spec_data
    assert_eq!(param[data_start], 0xAA);
    assert_eq!(param[data_start + 1], 0xBB);
}

// ==================== TPKT/COTP Header Tests ====================

#[test]
fn test_tpkt_header_construction() {
    let payload_len = 19; // COTP CC
    let tpkt_len = payload_len + 4;
    let tpkt: [u8; 4] = [
        0x03, 0x00,
        (tpkt_len >> 8) as u8,
        (tpkt_len & 0xFF) as u8,
    ];
    assert_eq!(tpkt[0], 0x03); // Version
    assert_eq!(tpkt[1], 0x00); // Reserved
    let parsed_len = ((tpkt[2] as usize) << 8) | tpkt[3] as usize;
    assert_eq!(parsed_len, 23); // 4 + 19
}

#[test]
fn test_cotp_cc_construction() {
    let cotp_cc: [u8; 19] = [
        0x0D,                   // CC type
        0x00, 0x13,             // LI = 19
        0x00, 0x00,             // Dest ref
        0x00, 0x01,             // Src ref
        0x00,                   // Class
        0xC0, 0x01, 0x0A,      // TPDU size: 1024
        0xC1, 0x02, 0x00, 0x01, // Calling TSAP
        0xC2, 0x02, 0x00, 0x00, // Called TSAP
    ];
    assert_eq!(cotp_cc[0], 0x0D); // CC type
    assert_eq!(cotp_cc[1], 0x00);
    assert_eq!(cotp_cc[2], 0x13); // LI = 19
}

#[test]
fn test_cotp_dt_header() {
    let cotp_dt: [u8; 3] = [0x02, 0xF0, 0x80];
    assert_eq!(cotp_dt[0], 0x02); // DT type
    assert_eq!(cotp_dt[1], 0xF0); // TPDU number with EOT
    assert_eq!(cotp_dt[2], 0x80); // Last data unit
}

// ==================== Edge Case Tests ====================

#[test]
fn test_zero_length_read() {
    let mem = create_test_memory();
    let result = mem.read(MemoryArea::DataBlocks, 401, 0, 0);
    // Reading 0 bytes should succeed with empty vec
    assert_eq!(result, Some(vec![]));
}

#[test]
fn test_write_then_overwrite() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 256);
    
    mem.write_dword(MemoryArea::DataBlocks, 1, 0, 0xDEADBEEF);
    assert_eq!(mem.read_dword(MemoryArea::DataBlocks, 1, 0), Some(0xDEADBEEF));
    
    // Overwrite
    mem.write_dword(MemoryArea::DataBlocks, 1, 0, 0xCAFEBABE);
    assert_eq!(mem.read_dword(MemoryArea::DataBlocks, 1, 0), Some(0xCAFEBABE));
}

#[test]
fn test_write_at_end_of_db() {
    let mut mem = PlcMemory::new();
    mem.add_db(1, 10);
    
    // Write at the very end
    assert!(mem.write(MemoryArea::DataBlocks, 1, 8, &[0xAA, 0xBB]));
    // One past the end
    assert!(!mem.write(MemoryArea::DataBlocks, 1, 9, &[0xCC, 0xDD]));
}

#[tokio::test]
async fn test_read_var_with_zero_items() {
    let sim = create_test_simulator();
    let param = vec![0x04, 0x00]; // Read with 0 items
    let resp = sim.handle_read_request(0x0001, &param).await.unwrap();
    
    // Should return valid response with no data items
    assert_eq!(resp[10], 0x04);
    assert_eq!(resp[11], 0x00);
}

#[test]
fn test_plc_memory_get_db_info() {
    let mem = create_test_memory();
    let info = mem.get_db_info(401).unwrap();
    assert_eq!(info.number, 401);
    assert_eq!(info.size, 38);
}

#[test]
fn test_plc_memory_get_db_info_nonexistent() {
    let mem = create_test_memory();
    assert!(mem.get_db_info(999).is_none());
}

#[test]
fn test_plc_memory_get_db_data() {
    let mem = create_test_memory();
    let data = mem.get_db_data(401).unwrap();
    assert_eq!(data.number, 401);
    assert_eq!(data.bytes.len(), 38);
}

#[test]
fn test_plc_memory_get_inputs_outputs_flags() {
    let mem = PlcMemory::new();
    assert_eq!(mem.get_inputs().len(), 256);
    assert_eq!(mem.get_outputs().len(), 256);
    assert_eq!(mem.get_flags().len(), 1024);
}
