# S7 PLC Simulator 测试规格文档

## 1. 概述

本文档定义 s7-plc-simulator 的完整单元测试规格，涵盖协议层、内存层和 API 层。

**已知待修复问题**：
- 模拟器在 TPKT 模式下发送 S7 响应时缺少 COTP DT 头（`02 F0 80`）
- s7connector-rs 期望格式：`TPKT header + COTP DT(3 bytes) + S7 PDU`
- 模拟器当前格式：`TPKT header + S7 PDU`（缺少 COTP DT）

---

## 2. 协议层测试 (src/lib.rs)

### 2.1 TPKT 帧解析测试

#### test_tpkt_header_parse_valid
```rust
#[test]
fn test_tpkt_header_parse_valid() {
    let header = [0x03, 0x00, 0x00, 0x1F]; // len = 31
    let len = ((header[2] as usize) << 8) | (header[3] as usize);
    assert_eq!(len, 31);
    assert_eq!(header[0], 0x03); // TPKT version
}
```

#### test_tpkt_header_parse_invalid_version
```rust
#[test]
fn test_tpkt_header_parse_invalid_version() {
    let header = [0x02, 0x00, 0x00, 0x10]; // version 0x02 is invalid
    assert_ne!(header[0], 0x03);
}
```

#### test_tpkt_payload_length_calculation
```rust
#[test]
fn test_tpkt_payload_length_calculation() {
    let tpkt_len: usize = 31;
    let payload_len = tpkt_len - 4;
    assert_eq!(payload_len, 27);
}
```

### 2.2 COTP 协议测试

#### test_cotp_cr_parsing
```rust
#[test]
fn test_cotp_cr_parsing() {
    // COTP CR: [LI, type=E0, dst_ref(2), src_ref(2), class(1), params...]
    let cotp_cr = [
        0x11, 0xE0, 0x00, 0x00, 0x00, 0x01, 0x00,
        0xC0, 0x01, 0x0A,
        0xC1, 0x02, 0x01, 0x00,
        0xC2, 0x02, 0x00, 0x00,
    ];
    assert_eq!(cotp_cr[0], 0x11); // LI field = 17
    assert_eq!(cotp_cr[1], 0xE0); // CR type
}
```

#### test_cotp_cc_construction
```rust
#[test]
fn test_cotp_cc_construction() {
    // COTP CC: [type=0D, LI=13, dst_ref(2), src_ref(2), class(1), params...]
    let cotp_cc = [
        0x0D, 0x00, 0x13, 0x00, 0x00, 0x00, 0x01, 0x00,
        0xC0, 0x01, 0x0A,
        0xC1, 0x02, 0x00, 0x01,
        0xC2, 0x02, 0x00, 0x00,
    ];
    assert_eq!(cotp_cc.len(), 19);
    assert_eq!(cotp_cc[0], 0x0D);
}
```

#### test_cotp_dt_format
```rust
#[test]
fn test_cotp_dt_format() {
    // COTP DT: [type=02, TPDU=F0, EOT=80, dst_ref(2), src_ref(2)]
    // Fixed 7 bytes, no LI field
    let cotp_dt = [0x02, 0xF0, 0x80, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(cotp_dt.len(), 7);
    assert_eq!(cotp_dt[0], 0x02); // DT type
    assert_eq!(cotp_dt[1], 0xF0); // TPDU number
    assert_eq!(cotp_dt[2], 0x80); // EOT flag
}
```

### 2.3 S7 PDU 测试

#### test_s7_pdu_header_structure
```rust
#[test]
fn test_s7_pdu_header_structure() {
    // S7 PDU Header (10 bytes, Big Endian):
    // [proto(1), type(1), rsv(2), ref(2), param_len(2), data_len(2)]
    let s7_header = [
        0x32, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x08, 0x00, 0x00,
    ];
    assert_eq!(s7_header.len(), 10);
    assert_eq!(s7_header[0], 0x32); // PROTOCOL_ID
    // param_len at bytes 6-7 = 8
    // data_len at bytes 8-9 = 0
}
```

#### test_s7_setup_communication_request
```rust
#[test]
fn test_s7_setup_communication_request() {
    // Setup Communication param (8 bytes):
    // [func=F0, reserved, max_amq_be(2), max_amq_le(2), pdu_len(2)]
    let setup_params = [
        0xF0, 0x00, 0x03, 0xE8, 0x03, 0xE8, 0x03, 0xE8,
    ];
    assert_eq!(setup_params.len(), 8);
    assert_eq!(setup_params[0], 0xF0); // Function code
}
```

#### test_s7_read_request_item_encoding
```rust
#[test]
fn test_s7_read_request_item_encoding() {
    // Read Request Item:
    // [spec=12, var_len, transport, reserved, num_hi, num_lo, db_hi, db_lo, area, off_hi, off_lo]
    let read_item = [
        0x12, 0x0A,       // spec, var_len
        0x10,             // transport size = BYTE
        0x00, 0x0A,       // number of elements = 10
        0x00, 0x01,       // DB number = 1
        0x84,             // area = DataBlocks
        0x00, 0x00,       // byte offset = 0
    ];
    assert_eq!(read_item.len(), 12);
    assert_eq!(read_item[0], 0x12); // Variable specification
    assert_eq!(read_item[2], 0x10); // Transport size BYTE
    assert_eq!(read_item[6], 0x84); // DataBlocks area
}
```

#### test_s7_write_request_item_encoding
```rust
#[test]
fn test_s7_write_request_item_encoding() {
    // Write Request Item (same as read, plus data after):
    let write_item = [
        0x12, 0x0A,       // spec, var_len
        0x10,             // transport size = BYTE
        0x00, 0x03,       // number of elements = 3
        0x00, 0x01,       // DB number = 1
        0x84,             // area = DataBlocks
        0x00, 0x00,       // byte offset = 0
        0x01, 0x02, 0x03, // data bytes
    ];
    assert_eq!(write_item.len(), 15);
}
```

### 2.4 握手流程测试

#### test_full_handshake_sequence
```rust
#[tokio::test]
async fn test_full_handshake_sequence() {
    // Test sequence:
    // 1. Client sends: COTP CR (TPKT wrapped)
    // 2. Server sends: COTP CC (TPKT wrapped)
    // 3. Server sends: COTP DT (TPKT wrapped)
    // 4. Client sends: COTP DT (TPKT wrapped)
    // 5. Client sends: S7 Setup Comm (COTP DT + S7, TPKT wrapped)
    // 6. Server sends: S7 Ack (COTP DT + S7, TPKT wrapped)

    // COTP CR in TPKT frame
    let cotp_cr = vec![
        0x03, 0x00, 0x00, 0x16, // TPKT header, len=22
        0x11, 0xE0, 0x00, 0x00, 0x00, 0x01, 0x00, // COTP CR
        0xC0, 0x01, 0x0A, 0xC1, 0x02, 0x01, 0x00,
        0xC2, 0x02, 0x00, 0x01,
    ];
    assert_eq!(cotp_cr.len(), 26);

    // Verify TPKT length field
    let tpkt_len = ((cotp_cr[2] as usize) << 8) | (cotp_cr[3] as usize);
    let payload_len = tpkt_len - 4;
    assert_eq!(payload_len, 22);
}
```

### 2.5 集成测试：完整 S7 Read 流程

#### test_s7_read_db100_integration ⚠️ 待修复
```rust
#[tokio::test]
async fn test_s7_read_db100_integration() {
    // This test documents the expected full S7 Read flow
    // that currently FAILS due to missing COTP DT in response

    // Step 1: TCP connect
    let mut stream = TcpStream::connect("127.0.0.1:102").await.unwrap();

    // Step 2: COTP CR
    let cotp_cr = build_cotp_cr_tpkt();
    stream.write_all(&cotp_cr).await.unwrap();

    // Step 3: Read COTP CC + COTP DT (two TPKT frames)
    let cc = read_tpkt_payload(&mut stream).await;
    let dt = read_tpkt_payload(&mut stream).await;
    assert_eq!(cc[0], 0x0D); // COTP CC
    assert_eq!(dt[0], 0x02); // COTP DT

    // Step 4: COTP DT activate
    let cotp_dt = build_cotp_dt_tpkt();
    stream.write_all(&cotp_dt).await.unwrap();

    // Step 5: S7 Setup Communication
    let setup = build_s7_setup_tpkt();
    stream.write_all(&setup).await.unwrap();

    // Step 6: Read S7 Ack
    let setup_resp = read_tpkt_payload(&mut stream).await;
    // EXPECTED: setup_resp = [02, F0, 80] + S7 AckData
    // ACTUAL:   setup_resp = S7 AckData only (missing COTP DT header!)
    assert_eq!(setup_resp[0], 0x02); // COTP DT
    assert_eq!(setup_resp[3], 0x32); // S7 protocol ID

    // Step 7: S7 Read DB100
    let read_req = build_s7_read_db100_tpkt();
    stream.write_all(&read_req).await.unwrap();

    // Step 8: Read S7 Read Response
    let read_resp = read_tpkt_payload(&mut stream).await;
    // EXPECTED: [02, F0, 80] + S7 AckData with DB100 data
    // ACTUAL: S7 AckData only (still missing COTP DT!)
    assert_eq!(read_resp[0], 0x02); // FAILS!
}
```

---

## 3. 内存层测试 (src/memory.rs)

### 3.1 数据块读写测试

#### test_plc_memory_new
```rust
#[test]
fn test_plc_memory_new() {
    let memory = PlcMemory::new();
    assert_eq!(memory.db_count(), 0);
    assert_eq!(memory.read(MemoryArea::Inputs, 0, 0, 1), Some(vec![0u8]));
}
```

#### test_plc_memory_init_default_db
```rust
#[test]
fn test_plc_memory_init_default_db() {
    let mut memory = PlcMemory::new();
    memory.init_default_db();

    assert!(memory.db_count() >= 5); // DB1, DB2, DB3, DB10, DB11, DB20, DB100
    assert!(memory.get_db_info(1).is_some());
    assert!(memory.get_db_info(100).is_some());
}
```

#### test_db_read_valid_range
```rust
#[test]
fn test_db_read_valid_range() {
    let mut memory = PlcMemory::new();
    memory.add_db(1, 256);
    memory.write(MemoryArea::DataBlocks, 1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

    let data = memory.read(MemoryArea::DataBlocks, 1, 0, 4).unwrap();
    assert_eq!(data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}
```

#### test_db_read_out_of_bounds
```rust
#[test]
fn test_db_read_out_of_bounds() {
    let mut memory = PlcMemory::new();
    memory.add_db(1, 256);

    // Read beyond DB size
    let data = memory.read(MemoryArea::DataBlocks, 1, 250, 20);
    assert!(data.is_none());
}
```

#### test_db_write_valid
```rust
#[test]
fn test_db_write_valid() {
    let mut memory = PlcMemory::new();
    memory.add_db(1, 256);

    let result = memory.write(MemoryArea::DataBlocks, 1, 0, &[0x12, 0x34, 0x56, 0x78]);
    assert!(result);

    let data = memory.read(MemoryArea::DataBlocks, 1, 0, 4).unwrap();
    assert_eq!(data, vec![0x12, 0x34, 0x56, 0x78]);
}
```

#### test_db_write_out_of_bounds
```rust
#[test]
fn test_db_write_out_of_bounds() {
    let mut memory = PlcMemory::new();
    memory.add_db(1, 10);

    // Write beyond DB size
    let result = memory.write(MemoryArea::DataBlocks, 1, 8, &[1, 2, 3, 4]);
    assert!(!result);
}
```

#### test_db_clear
```rust
#[test]
fn test_db_clear() {
    let mut memory = PlcMemory::new();
    memory.add_db(1, 256);
    memory.write(MemoryArea::DataBlocks, 1, 0, &[0xFF; 256]).unwrap();

    memory.clear_db(1).unwrap();

    let data = memory.read(MemoryArea::DataBlocks, 1, 0, 256).unwrap();
    assert!(data.iter().all(|&b| b == 0));
}
```

### 3.2 内存区域测试

#### test_area_inputs
```rust
#[test]
fn test_area_inputs() {
    let mut memory = PlcMemory::new();
    assert!(memory.write(MemoryArea::Inputs, 0, 0, &[0xAB]).unwrap());
    let data = memory.read(MemoryArea::Inputs, 0, 0, 1).unwrap();
    assert_eq!(data, vec![0xAB]);
}
```

#### test_area_outputs
```rust
#[test]
fn test_area_outputs() {
    let mut memory = PlcMemory::new();
    assert!(memory.write(MemoryArea::Outputs, 0, 0, &[0xCD]).unwrap());
    let data = memory.read