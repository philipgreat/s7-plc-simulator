# S7 PLC Simulator

A simulated S7 PLC for testing [s7-connector-rs](https://github.com/philipgreat/s7-connector-rs).

## Features

- Simulates S7-300/400/1200/1500 PLCs
- Maintains internal state (memory areas)
- Supports TCP/IP connections on port 102
- Pre-loaded data blocks for testing
- Full S7 protocol handshake (COTP + S7)

## Memory Areas

- **Inputs (I)** - 256 bytes
- **Outputs (Q)** - 256 bytes
- **Flags (M)** - 1024 bytes
- **Data Blocks (DB)** - Multiple pre-loaded DBs

## Pre-loaded Data Blocks

| DB | Size | Content |
|----|------|---------|
| DB1 | 256 bytes | General data (zeros) |
| DB2 | 128 bytes | Counter values |
| DB3 | 128 bytes | Timer values |
| DB10 | 64 bytes | Real values: [1.5, 2.5, 3.14, 100.0] |
| DB11 | 32 bytes | Integer values: [100, -200, 300, -400, 500, -600, 700, -800] |
| DB20 | 128 bytes | String: "Hello World!" |

## Installation

```bash
cargo build --release
```

## Usage

```bash
# Default (S7-300, port 102)
cargo run

# Custom settings
cargo run -- --port 102 --plc-type S7-1500 --rack 0 --slot 1 --verbose

# Or use the binary
./target/release/s7-plc-simulator --port 102 --plc-type S7-300
```

## Command Line Options

```
-s, --port <PORT>      Port to listen on (default: 102)
-t, --plc-type <TYPE>  PLC type (default: S7-300)
-r, --rack <RACK>      Rack number (default: 0)
-s, --slot <SLOT>      Slot number (default: 2)
-v, --verbose          Verbose output
```

## Testing with s7-connector-rs

Start the simulator:
```bash
cargo run
```

In another terminal, test with s7-connector-rs:
```bash
cd ../s7-connector-rs
cargo run --example read_db
```

## Protocol Support

- [x] COTP Connection Request/Confirm
- [x] S7 Setup Communication
- [x] S7 Read (Function 0x04)
- [x] S7 Write (Function 0x05)
- [ ] S7 Variable List Read
- [ ] S7 Variable List Write
- [ ] Block operations
- [ ] Date/Time operations

## License

MIT
