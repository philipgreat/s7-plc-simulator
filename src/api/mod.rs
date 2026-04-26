//!
//! REST API for PLC Memory Management
//! 
//! Provides HTTP endpoints for managing data blocks and memory areas

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post, put, delete},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::memory::{SharedMemory, MemoryArea, PlcMemory, DataBlock, VariableDefinition};
use crate::{ConnectionList, ClientConnection};

/// Application state
#[derive(Clone)]
pub struct AppState {
    pub memory: SharedMemory,
    pub s7_port: u16,
    pub connections: ConnectionList,
}

/// Health check response
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub memory_blocks: usize,
    pub s7_port: u16,
}

/// List all data blocks
#[derive(Serialize)]
pub struct DbListResponse {
    pub count: usize,
    pub data_blocks: Vec<DbInfo>,
}

#[derive(Serialize)]
pub struct DbInfo {
    pub number: u16,
    pub size: usize,
}

/// Get single data block
#[derive(Serialize)]
pub struct DbResponse {
    pub number: u16,
    pub size: usize,
    pub description: Option<String>,
    pub hex: String,
    pub data: Vec<DbDataItem>,
    pub variables: Vec<VariableWithValue>,
}

#[derive(Serialize)]
pub struct VariableWithValue {
    pub name: String,
    pub offset: usize,
    #[serde(rename = "type")]
    pub data_type: String,
    pub value: serde_json::Value,
    pub raw_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<std::collections::HashMap<String, String>>,
}

/// Get data block with variables (for internal use)
#[derive(Serialize)]
pub struct DbWithVariables {
    pub number: u16,
    pub size: usize,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub variables: Vec<VariableDefinition>,
}

#[derive(Serialize)]
pub struct DbDataItem {
    pub offset: usize,
    pub hex: String,
    pub bytes: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_int: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_real: Option<f32>,
}

/// Create new data block request
#[derive(Deserialize)]
pub struct CreateDbRequest {
    pub number: u16,
    pub size: usize,
}

/// Write data request
#[derive(Deserialize)]
pub struct WriteDataRequest {
    pub offset: usize,
    pub value: String, // hex string or decimal
    pub data_type: String, // "byte", "word", "dword", "int", "real", "string"
}

/// Write multiple values request
#[derive(Deserialize)]
pub struct WriteMultiRequest {
    pub values: Vec<WriteDataItem>,
}

#[derive(Deserialize)]
pub struct WriteDataItem {
    pub offset: usize,
    pub value: String,
    pub data_type: String,
}

// ===== Handlers =====

/// GET / - Serve admin page
pub async fn serve_admin() -> Html<&'static str> {
    Html(include_str!("../static/admin.html"))
}

/// GET /health - Health check
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let memory = state.memory.read().unwrap();
    Json(HealthResponse {
        status: "ok".to_string(),
        memory_blocks: memory.db_count(),
        s7_port: state.s7_port,
    })
}

/// GET /api/dbs - List all data blocks
pub async fn list_dbs(State(state): State<AppState>) -> Json<DbListResponse> {
    let memory = state.memory.read().unwrap();
    let dbs = memory.list_dbs();
    Json(DbListResponse {
        count: dbs.len(),
        data_blocks: dbs.iter().map(|db| DbInfo {
            number: db.number,
            size: db.size,
        }).collect(),
    })
}

/// GET /api/db/:number - Get data block content
pub async fn get_db(
    State(state): State<AppState>,
    Path(db_number): Path<u16>,
) -> impl IntoResponse {
    let memory = state.memory.read().unwrap();
    
    match memory.get_db_info(db_number) {
        Some(db_info) => {
            let db_data = memory.get_db_data(db_number);
            let bytes = memory.read(MemoryArea::DataBlocks, db_number, 0, db_info.size)
                .unwrap_or_default();
            
            // Format data for display
            let mut items = Vec::new();
            for chunk in bytes.chunks(16) {
                let offset = items.len() * 16;
                let hex: String = chunk.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                let as_int = memory.read_int(MemoryArea::DataBlocks, db_number, offset);
                let as_real = memory.read_real(MemoryArea::DataBlocks, db_number, offset);
                
                items.push(DbDataItem {
                    offset,
                    hex,
                    bytes: chunk.to_vec(),
                    as_int,
                    as_real,
                });
            }
            
            // Format variables with current values
            let variables = if let Some(db) = db_data {
                db.variables.iter().map(|var| {
                    let value = db.get_variable_value(var);
                    let raw_bytes = db.bytes.get(var.offset..var.offset + get_type_size(&var.data_type)).unwrap_or(&[]);
                    VariableWithValue {
                        name: var.name.clone(),
                        offset: var.offset,
                        data_type: var.data_type.clone(),
                        value,
                        raw_hex: raw_bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "),
                        unit: var.unit.clone(),
                        description: var.description.clone(),
                        enum_values: var.enum_values.clone(),
                    }
                }).collect()
            } else {
                Vec::new()
            };
            
            Json(DbResponse {
                number: db_info.number,
                size: db_info.size,
                description: db_info.description.clone(),
                hex: hex::encode(&bytes),
                data: items,
                variables,
            }).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": format!("DB{}", db_number)
        }))).into_response(),
    }
}

/// Get size in bytes for a data type
fn get_type_size(data_type: &str) -> usize {
    match data_type.to_uppercase().as_str() {
        "BOOL" => 1,
        "BYTE" => 1,
        "WORD" => 2,
        "DWORD" => 4,
        "INT" => 2,
        "DINT" => 4,
        "REAL" => 4,
        "STRING" => 258, // 4 header + 254 max
        "DT" | "DATE_AND_TIME" => 8,
        _ => 1,
    }
}

/// POST /api/db - Create new data block
pub async fn create_db(
    State(state): State<AppState>,
    Json(req): Json<CreateDbRequest>,
) -> impl IntoResponse {
    info!("Creating DB{} with size {}", req.number, req.size);
    
    let mut memory = state.memory.write().unwrap();
    memory.add_db(req.number, req.size);
    
    (StatusCode::CREATED, Json(serde_json::json!({
        "success": true,
        "db": req.number,
        "size": req.size
    })))
}

/// DELETE /api/db/:number - Delete data block
pub async fn delete_db(
    State(state): State<AppState>,
    Path(db_number): Path<u16>,
) -> impl IntoResponse {
    info!("Deleting DB{}", db_number);
    
    let mut memory = state.memory.write().unwrap();
    if memory.remove_db(db_number) {
        (StatusCode::OK, Json(serde_json::json!({
            "success": true,
            "db": db_number
        }))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": format!("DB{} not found", db_number)
        }))).into_response()
    }
}

/// POST /api/db/:number/write - Write data to DB
pub async fn write_db(
    State(state): State<AppState>,
    Path(db_number): Path<u16>,
    Json(req): Json<WriteDataRequest>,
) -> impl IntoResponse {
    let mut memory = state.memory.write().unwrap();
    
    let result = match req.data_type.as_str() {
        "byte" => {
            let value = u8::from_str_radix(&req.value, 16).unwrap_or_else(|_| req.value.parse().unwrap_or(0));
            memory.write_byte(MemoryArea::DataBlocks, db_number, req.offset, value)
        }
        "word" => {
            let value = u16::from_str_radix(&req.value, 16).unwrap_or_else(|_| req.value.parse().unwrap_or(0));
            memory.write_word(MemoryArea::DataBlocks, db_number, req.offset, value)
        }
        "dword" => {
            let value = u32::from_str_radix(&req.value, 16).unwrap_or_else(|_| req.value.parse().unwrap_or(0));
            memory.write_dword(MemoryArea::DataBlocks, db_number, req.offset, value)
        }
        "int" => {
            let value: i16 = req.value.parse().unwrap_or(0);
            memory.write_int(MemoryArea::DataBlocks, db_number, req.offset, value)
        }
        "real" => {
            let value: f32 = req.value.parse().unwrap_or(0.0);
            memory.write_real(MemoryArea::DataBlocks, db_number, req.offset, value)
        }
        "string" => {
            memory.write_string(db_number, req.offset, &req.value)
        }
        "hex" => {
            // Parse hex string
            let bytes = hex::decode(&req.value.replace(" ", "")).unwrap_or_default();
            memory.write(MemoryArea::DataBlocks, db_number, req.offset, &bytes)
        }
        _ => false,
    };
    
    if result {
        (StatusCode::OK, Json(serde_json::json!({
            "success": true,
            "db": db_number,
            "offset": req.offset
        }))).into_response()
    } else {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Write failed"
        }))).into_response()
    }
}

/// POST /api/db/:number/write-multi - Write multiple values
pub async fn write_db_multi(
    State(state): State<AppState>,
    Path(db_number): Path<u16>,
    Json(req): Json<WriteMultiRequest>,
) -> impl IntoResponse {
    let mut memory = state.memory.write().unwrap();
    let mut success_count = 0;
    let total = req.values.len();
    
    for item in req.values {
        let result = match item.data_type.as_str() {
            "byte" => {
                let value = u8::from_str_radix(&item.value, 16).unwrap_or_else(|_| item.value.parse().unwrap_or(0));
                memory.write_byte(MemoryArea::DataBlocks, db_number, item.offset, value)
            }
            "word" => {
                let value = u16::from_str_radix(&item.value, 16).unwrap_or_else(|_| item.value.parse().unwrap_or(0));
                memory.write_word(MemoryArea::DataBlocks, db_number, item.offset, value)
            }
            "dword" => {
                let value = u32::from_str_radix(&item.value, 16).unwrap_or_else(|_| item.value.parse().unwrap_or(0));
                memory.write_dword(MemoryArea::DataBlocks, db_number, item.offset, value)
            }
            "int" => {
                let value: i16 = item.value.parse().unwrap_or(0);
                memory.write_int(MemoryArea::DataBlocks, db_number, item.offset, value)
            }
            "real" => {
                let value: f32 = item.value.parse().unwrap_or(0.0);
                memory.write_real(MemoryArea::DataBlocks, db_number, item.offset, value)
            }
            "string" => {
                memory.write_string(db_number, item.offset, &item.value)
            }
            "hex" => {
                let bytes = hex::decode(&item.value.replace(" ", "")).unwrap_or_default();
                memory.write(MemoryArea::DataBlocks, db_number, item.offset, &bytes)
            }
            _ => false,
        };
        if result {
            success_count += 1;
        }
    }
    
    (StatusCode::OK, Json(serde_json::json!({
        "success": success_count,
        "total": total
    })))
}

/// POST /api/db/:number/clear - Clear data block
pub async fn clear_db(
    State(state): State<AppState>,
    Path(db_number): Path<u16>,
) -> impl IntoResponse {
    let mut memory = state.memory.write().unwrap();
    
    if memory.clear_db(db_number) {
        (StatusCode::OK, Json(serde_json::json!({
            "success": true,
            "db": db_number
        }))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": format!("DB{} not found", db_number)
        }))).into_response()
    }
}

/// GET /api/memory/inputs - Get inputs
pub async fn get_inputs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let memory = state.memory.read().unwrap();
    let bytes = memory.get_inputs();
    Json(serde_json::json!({
        "area": "Inputs",
        "size": bytes.len(),
        "hex": hex::encode(&bytes)
    }))
}

/// GET /api/memory/outputs - Get outputs
pub async fn get_outputs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let memory = state.memory.read().unwrap();
    let bytes = memory.get_outputs();
    Json(serde_json::json!({
        "area": "Outputs",
        "size": bytes.len(),
        "hex": hex::encode(&bytes)
    }))
}

/// GET /api/memory/flags - Get flags
pub async fn get_flags(State(state): State<AppState>) -> Json<serde_json::Value> {
    let memory = state.memory.read().unwrap();
    let bytes = memory.get_flags();
    Json(serde_json::json!({
        "area": "Flags",
        "size": bytes.len(),
        "hex": hex::encode(&bytes)
    }))
}

/// GET /api/connections - List active S7 client connections
pub async fn list_connections(State(state): State<AppState>) -> Json<serde_json::Value> {
    let conns = state.connections.read().unwrap();
    Json(serde_json::json!({
        "count": conns.len(),
        "connections": conns.clone()
    }))
}

/// Create the router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_admin))
        .route("/health", get(health))
        .route("/api/dbs", get(list_dbs))
        .route("/api/db/:number", get(get_db))
        .route("/api/db", post(create_db))
        .route("/api/db/:number/write", post(write_db))
        .route("/api/db/:number/write-multi", post(write_db_multi))
        .route("/api/db/:number/clear", post(clear_db))
        .route("/api/db/:number", delete(delete_db))
        .route("/api/memory/inputs", get(get_inputs))
        .route("/api/memory/outputs", get(get_outputs))
        .route("/api/memory/flags", get(get_flags))
        .route("/api/connections", get(list_connections))
        .with_state(state)
}

/// Start the web server
pub async fn start_server(port: u16, memory: SharedMemory, s7_port: u16, connections: ConnectionList) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState {
        memory,
        s7_port,
        connections,
    };
    
    let addr = format!("0.0.0.0:{}", port);
    info!("Starting Web API on http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, create_router(state)).await?;
    
    Ok(())
}
