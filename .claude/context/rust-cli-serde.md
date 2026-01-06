# Rust Serde Patterns for CLI Applications

This document provides guidance on using serde for serialization/deserialization in Rust CLI applications. These patterns apply to JSON config files, API responses, and CLI output formatting.

## Basic Derive Patterns

### Output-Only Types (Serialize)

Use `Serialize` for types that are only written out (CLI output, file exports):

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct CommandOutput {
    pub success: bool,
    pub message: String,
    pub items_processed: usize,
}
```

### Input-Only Types (Deserialize)

Use `Deserialize` for types that are only read in (API responses, config files):

```rust
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ApiResponse {
    pub id: String,
    pub data: Vec<String>,
    pub metadata: Option<Metadata>,
}
```

### Bidirectional Types (Both)

Use both derives for types that are read and written (cache files, state persistence):

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct AppState {
    pub version: String,
    pub last_run: Option<String>,
    pub settings: Settings,
}
```

## Field Renaming

### Single Field Rename

Rename individual fields to match external formats:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "userId")]
    pub user_id: String,
    
    #[serde(rename = "firstName")]
    pub first_name: String,
    
    #[serde(rename = "lastName")]
    pub last_name: String,
}
```

### Container-Level Rename

Apply naming convention to all fields in a struct:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiRequest {
    pub user_id: String,      // Serializes as "userId"
    pub request_time: String, // Serializes as "requestTime"
    pub page_size: usize,     // Serializes as "pageSize"
}
```

Common `rename_all` values:
- `"camelCase"` - JavaScript/JSON APIs
- `"snake_case"` - Python APIs, databases
- `"PascalCase"` - C#/.NET APIs
- `"SCREAMING_SNAKE_CASE"` - Constants
- `"kebab-case"` - CLI flags, URLs

## Default Values

### Using the Default Trait

Apply defaults for missing fields during deserialization:

```rust
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub name: String,
    
    #[serde(default)]
    pub verbose: bool, // Defaults to false if missing
    
    #[serde(default)]
    pub retries: u32, // Defaults to 0 if missing
    
    #[serde(default)]
    pub tags: Vec<String>, // Defaults to empty vec if missing
}
```

### Custom Default Functions

Provide custom default values:

```rust
use serde::Deserialize;

fn default_timeout() -> u64 {
    30
}

fn default_port() -> u16 {
    8080
}

#[derive(Deserialize)]
pub struct ServerConfig {
    pub host: String,
    
    #[serde(default = "default_port")]
    pub port: u16,
    
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
```

## Skip Serialization

### Skip None Values

Omit `None` values from serialized output for cleaner JSON:

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}
```

### Skip Empty Collections

Omit empty vectors or maps:

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct Report {
    pub name: String,
    
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}
```

### Skip Based on Custom Condition

```rust
use serde::Serialize;

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Serialize)]
pub struct Stats {
    pub total: u32,
    
    #[serde(skip_serializing_if = "is_zero")]
    pub failed: u32,
}
```

## Tagged Enums

### Internally Tagged (Discriminated Unions)

Use `tag` for JSON with a type discriminator field:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "user_created")]
    UserCreated { user_id: String, email: String },
    
    #[serde(rename = "user_deleted")]
    UserDeleted { user_id: String },
    
    #[serde(rename = "login")]
    Login { user_id: String, ip_address: String },
}

// Serializes to: {"type": "user_created", "user_id": "123", "email": "..."}
```

### Adjacently Tagged

Use `tag` and `content` for separate type and data fields:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Text(String),
    Image { url: String, alt: String },
    File { path: String, size: u64 },
}

// Serializes to: {"type": "Text", "data": "hello"}
// Or: {"type": "Image", "data": {"url": "...", "alt": "..."}}
```

### Untagged

Match based on structure (useful for polymorphic APIs):

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiValue {
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<ApiValue>),
}
```

## JSON Handling with serde_json

### Dynamic Data with serde_json::Value

Handle unknown or variable JSON structures:

```rust
use serde_json::Value;

fn process_dynamic_response(json_str: &str) -> Result<(), serde_json::Error> {
    let value: Value = serde_json::from_str(json_str)?;
    
    // Access fields dynamically
    if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
        println!("Name: {}", name);
    }
    
    // Check types
    if value["count"].is_number() {
        let count = value["count"].as_i64().unwrap_or(0);
        println!("Count: {}", count);
    }
    
    Ok(())
}
```

### The json! Macro

Build JSON values inline:

```rust
use serde_json::json;

fn create_request_body(user_id: &str, action: &str) -> serde_json::Value {
    json!({
        "user_id": user_id,
        "action": action,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "metadata": {
            "source": "cli",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}
```

### Serialization Functions

```rust
use serde::Serialize;

#[derive(Serialize)]
struct Output {
    status: String,
    count: usize,
}

fn output_json(data: &Output, pretty: bool) -> Result<String, serde_json::Error> {
    if pretty {
        serde_json::to_string_pretty(data)
    } else {
        serde_json::to_string(data)
    }
}
```

### Deserialization Functions

```rust
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Config {
    name: String,
    enabled: bool,
}

// From string
fn parse_config(json_str: &str) -> Result<Config, serde_json::Error> {
    serde_json::from_str(json_str)
}

// From Value (already parsed JSON)
fn config_from_value(value: Value) -> Result<Config, serde_json::Error> {
    serde_json::from_value(value)
}

// From reader (file, network stream)
fn config_from_reader<R: std::io::Read>(reader: R) -> Result<Config, serde_json::Error> {
    serde_json::from_reader(reader)
}
```

## Type Separation Pattern

Separate API types from domain types following the Functional Core - Imperative Shell pattern.

### API Response Types (Deserialize Only)

Define types that match external API structures exactly:

```rust
use serde::Deserialize;

/// Raw API response - matches external JSON structure exactly
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiUserResponse {
    pub user_id: String,
    pub display_name: String,
    pub email_address: Option<String>,
    pub created_at: String,
    pub is_active: bool,
    pub role_ids: Vec<i32>,
}
```

### Domain Output Types (Serialize Only)

Define types optimized for CLI output:

```rust
use serde::Serialize;

/// Domain type - optimized for CLI output
#[derive(Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    
    pub active: bool,
    pub role_count: usize,
}
```

### Pure Transformation Functions

Convert between types with pure functions (no I/O):

```rust
/// Pure transformation - no side effects, easily testable
pub fn api_response_to_user(response: ApiUserResponse) -> User {
    User {
        id: response.user_id,
        name: response.display_name,
        email: response.email_address,
        active: response.is_active,
        role_count: response.role_ids.len(),
    }
}

/// Transform collections
pub fn api_responses_to_users(responses: Vec<ApiUserResponse>) -> Vec<User> {
    responses.into_iter().map(api_response_to_user).collect()
}
```

### Complete Example

```rust
use serde::{Deserialize, Serialize};

// === API Layer (Deserialize) ===
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTask {
    pub task_id: String,
    pub task_name: String,
    pub assigned_to: Option<String>,
    pub status_code: i32,
    pub created_timestamp: String,
}

// === Domain Layer (Serialize) ===
#[derive(Serialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    
    pub status: TaskStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Complete,
    Failed,
}

// === Transformation (Pure Functions) ===
fn status_from_code(code: i32) -> TaskStatus {
    match code {
        0 => TaskStatus::Pending,
        1 => TaskStatus::InProgress,
        2 => TaskStatus::Complete,
        _ => TaskStatus::Failed,
    }
}

pub fn api_task_to_task(api: ApiTask) -> Task {
    Task {
        id: api.task_id,
        name: api.task_name,
        assignee: api.assigned_to,
        status: status_from_code(api.status_code),
    }
}
```

## Local Deserialize Structs

Define structs inline when parsing specific JSON structures that don't need reuse:

```rust
use serde::Deserialize;

pub fn extract_version_from_response(json: &str) -> Result<String, serde_json::Error> {
    // Local struct - only used here
    #[derive(Deserialize)]
    struct VersionResponse {
        version: String,
    }
    
    let response: VersionResponse = serde_json::from_str(json)?;
    Ok(response.version)
}

pub fn parse_paginated_ids(json: &str) -> Result<Vec<String>, serde_json::Error> {
    // Local structs for nested structure
    #[derive(Deserialize)]
    struct Item {
        id: String,
    }
    
    #[derive(Deserialize)]
    struct PaginatedResponse {
        items: Vec<Item>,
        #[allow(dead_code)]
        next_page: Option<String>,
    }
    
    let response: PaginatedResponse = serde_json::from_str(json)?;
    Ok(response.items.into_iter().map(|i| i.id).collect())
}
```

### When to Use Local Structs

- One-time parsing of specific API endpoints
- Extracting subset of fields from large responses
- Intermediate parsing before transformation
- Test fixtures and mocks

## Quick Reference

| Attribute | Purpose | Example |
|-----------|---------|---------|
| `#[derive(Serialize)]` | Enable serialization | Output types |
| `#[derive(Deserialize)]` | Enable deserialization | Input/API types |
| `#[serde(rename = "x")]` | Rename single field | `#[serde(rename = "userId")]` |
| `#[serde(rename_all = "x")]` | Rename all fields | `#[serde(rename_all = "camelCase")]` |
| `#[serde(default)]` | Use Default if missing | Optional config fields |
| `#[serde(default = "fn")]` | Custom default function | `#[serde(default = "default_port")]` |
| `#[serde(skip_serializing_if = "fn")]` | Conditionally skip | `#[serde(skip_serializing_if = "Option::is_none")]` |
| `#[serde(tag = "x")]` | Internal enum tag | `#[serde(tag = "type")]` |
| `#[serde(tag = "x", content = "y")]` | Adjacent enum tag | `#[serde(tag = "type", content = "data")]` |
| `#[serde(untagged)]` | No enum tag | Match by structure |
| `#[serde(flatten)]` | Flatten nested struct | Embed fields inline |
| `#[serde(skip)]` | Always skip field | Internal state |
| `#[serde(alias = "x")]` | Accept alternate name | `#[serde(alias = "user_id")]` |

## Common Patterns Summary

```rust
// API response (external format)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse { /* ... */ }

// CLI output (clean JSON)
#[derive(Serialize)]
struct Output {
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
}

// Config file (with defaults)
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    optional: bool,
}

// Tagged enum (discriminated union)
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum Event { /* ... */ }
```
