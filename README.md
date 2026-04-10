# edgy-s

[中文文档](./README_ZH.md)

A minimalist WebSocket/HTTP bidirectional RPC framework for building complex microservice applications with elegant, function-based routing.

## Features

- **Minimalist API** - Bind functions as routes with a single call
- **Bidirectional RPC** - Both client and server can initiate remote calls
- **HTTP Support** - Full HTTP request/response handling with streaming support
- **Type Safe** - Strict type constraints and full serde-based serialization
- **Automatic Path Derivation** - Routes are auto-generated from function names
- **Zero Boilerplate** - No macros, no complex configuration
- **Feature Flags** - Include only what you need (client/server)
- **Auto Reconnection** - WebSocket client with configurable retry logic
- **Builder Pattern** - Flexible client/service configuration
- **Shared State** - Built-in state management with `Arc<RwLock<S>>` for concurrent access

## Installation

```toml
[dependencies]
edgy-s = { version = "1.1", features = ["server", "client"] }
```

## Quick Start

### Server Example

```rust
use edgy_s::{
    Binding, HttpServerAsyncFn, WsAsyncFn,
    server::{EdgyService, HttpAccessor, WsAccessor, WsCaller},
};
use async_stream::stream;
use futures_util::{Stream, StreamExt};
use std::{io::Result as IoResult, pin::Pin};
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create server with builder pattern
    let service = EdgyService::builder("0.0.0.0:8080")
        .workers(4)
        .build()
        .await?;
    
    // Bind WebSocket route with lifecycle hooks
    let bd_api_add = api_add
        .bind(&service)
        .await?
        .on_open(on_open)
        .await
        .on_close(on_close)
        .await;
    
    // Bind HTTP routes
    let bd_index = index.bind_as_response(&service).await?;
    let bd_stream = countdown.bind_as_response(&service).await?;
    
    service.run().await?;
    
    // Cleanup
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;
    bd_stream.unbind().await?;
    
    Ok(())
}

// WebSocket handler - bidirectional RPC
async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    // Server can call client methods!
    tokio::spawn(async move {
        let result: i32 = (5, 5).call_remotely(&accessor).await.unwrap();
        println!("Client computed: 5 + 5 = {}", result);
    });
    a + b
}

async fn on_open(accessor: HttpAccessor) {
    println!("WebSocket opened from: {}", accessor.get_addr());
}

async fn on_close(accessor: WsAccessor) {
    println!("WebSocket closed from: {}", accessor.get_addr());
}

// HTTP handler - simple response
async fn index(accessor: HttpAccessor, body: String) -> String {
    let name = accessor.get_argument("name").unwrap_or_default();
    accessor.set_header("Content-Type", "text/html").unwrap();
    format!("<html><body>Hello {}, {}!</body></html>", name, body)
}

// HTTP handler - streaming response
async fn countdown(accessor: HttpAccessor, _body: String) -> Pin<Box<impl Stream<Item = String>>> {
    let from = accessor.get_argument("from")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10u8);
    
    Box::pin(stream! {
        yield format!("<p>Countdown from {}</p>", from);
        for i in (0..from).rev() {
            sleep(Duration::from_secs(1)).await;
            yield format!("<p>{}</p>", i);
        }
    })
}
```

### Client Example

```rust
use edgy_s::{
    Binding, HttpClientAsyncFn, WsAsyncFn,
    client::{EdgyClient, HttpPost, RequestAccessor, WsAccessor, WsCaller},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create client with builder pattern
    let client = EdgyClient::builder("ws://localhost:8080")?
        .workers(2)
        .max_retries(5)
        .retry_interval_ms(1000)
        .build()?;
    
    // Bind WebSocket route
    let bd_api_add = api_add
        .bind(&client)
        .await?
        .on_open(on_open)
        .await
        .on_close(on_close)
        .await;
    
    // Bind HTTP route
    let bd_index = index.bind_as_request(&client).await?;
    
    // Make HTTP POST request
    tokio::spawn(async {
        let (response, accessor): (String, _) = "request body"
            .post(index)
            .await
            .unwrap();
        println!("Status: {}, Response: {}", accessor.status(), response);
    });
    
    // Call server via WebSocket RPC
    tokio::spawn(async {
        let result: i32 = (1, 2).call_remotely(api_add).await.unwrap();
        println!("Server computed: 1 + 2 = {}", result);
    });
    
    client.run().await?;
    
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;
    
    Ok(())
}

// WebSocket handler - receives calls from server
async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    println!("Server requested: {} + {}", a, b);
    a + b
}

// HTTP request configurator
async fn index(accessor: RequestAccessor) {
    accessor.set_header("User-Agent", "edgy-s-client").unwrap();
    accessor.set_argument("name", "world");
}

async fn on_open(accessor: WsAccessor) {
    println!("Connected to: {}", accessor.path());
}

async fn on_close(accessor: WsAccessor) {
    println!("Disconnected from: {}", accessor.path());
}
```

## API Reference

### Client Configuration

```rust
let client = EdgyClient::builder("ws://localhost:8080")?
    .workers(4)              // Number of async worker threads
    .max_retries(5)          // Max WebSocket reconnection attempts
    .retry_interval_ms(1000) // Milliseconds between retries
    .retry_interval(Duration::from_secs(1)) // Or use Duration
    .build()?;
```

### Server Configuration

```rust
let service = EdgyService::builder("0.0.0.0:8080")
    .workers(4)
    .build()
    .await?;
```

### HTTP Methods

```rust
// GET request
let (body, accessor): (String, _) = ().get(handler).await?;

// POST request
let (body, accessor): (String, _) = "body".post(handler).await?;

// PUT request
let (body, accessor): (String, _) = "body".put(handler).await?;

// PATCH request
let (body, accessor): (String, _) = "body".patch(handler).await?;

// DELETE request
let (body, accessor): (String, _) = ().delete(handler).await?;

// HEAD request
let accessor: HttpAccessor = ().head(handler).await?;
```

### Accessor Methods

#### Server-side (WsAccessor / HttpAccessor)

| Method | Description |
|--------|-------------|
| `get_addr()` | Get client socket address |
| `get_argument(name)` | Get URL query parameter (decoded) |
| `get_arguments(name)` | Get all values for a parameter |
| `get_all_arguments()` | Get all query parameters as HashMap |
| `get_header(name)` | Get request header |
| `get_headers()` | Get all request headers |
| `set_header(name, value)` | Set response header |
| `add_header(name, value)` | Append response header |
| `get_other_conns()` | Get other connections to same path (WsAccessor only) |
| `find_conn(target, predicate)` | Find a WebSocket connection to a specific path matching predicate (WsAccessor only) |
| `find_ws_conn(target, predicate)` | Find a WebSocket connection to a specific path matching predicate (HttpAccessor only) |

#### Client-side (WsAccessor / RequestAccessor)

| Method | Description |
|--------|-------------|
| `path()` | Get the request path |
| `status()` | Get HTTP status code (WsAccessor after connection) |
| `get_header(name)` | Get response header |
| `get_headers()` | Get all response headers |
| `set_header(name, value)` | Set request header (RequestAccessor) |
| `add_header(name, value)` | Append request header (RequestAccessor) |
| `set_argument(name, value)` | Set URL query parameter (RequestAccessor) |

## Breaking Changes in 1.0

### API Renames

| 0.x | 1.0 |
|-----|-----|
| `AsyncFun` | `WsAsyncFn` |
| `ServiceAccessor` | `WsAccessor` (server) |
| `ClientAccessor` | `WsAccessor` (client) |
| `ClientCaller` / `ServiceCaller` | `WsCaller` |

### Constructor Changes

```rust
// 0.x
let client = EdgyClient::new("ws://localhost", 4)?;
let service = EdgyService::new("0.0.0.0:80", 4).await?;

// 1.0
let client = EdgyClient::builder("ws://localhost")?
    .workers(4)
    .max_retries(5)
    .build()?;
let service = EdgyService::builder("0.0.0.0:80")
    .workers(4)
    .build()
    .await?;
```

### New HTTP Support

1.0 adds comprehensive HTTP support for both client and server:

- Server: `bind_as_response()`, `bind_by_path_as_response()`
- Client: `bind_as_request()`, `bind_by_path_as_request()`
- HTTP methods: `get()`, `post()`, `put()`, `patch()`, `delete()`, `head()`
- Streaming request/response bodies

### Lifecycle Hooks

```rust
// 0.x - no lifecycle hooks

// 1.0 - chain lifecycle handlers
binding
    .on_open(handler)
    .await
    .on_close(handler)
    .await
```

## Request ID Configuration

Choose the appropriate request ID width based on your concurrency needs:

| Feature | Type | Max Concurrent Requests |
|---------|------|------------------------|
| `req_id_u8` (default) | u8 | 256 |
| `req_id_u16` | u16 | 65,536 |
| `req_id_u32` | u32 | ~4.2 billion |
| `req_id_u64` | u64 | Virtually unlimited |

```toml
[dependencies]
edgy-s = { version = "1.1", features = ["server", "client", "req_id_u32"] }
```

## Shared State Management

`EdgyService` and `EdgyClient` support shared state for managing application data across connections.

### Server with State

```rust
use edgy_s::{
    Binding, WsAsyncFn,
    server::{EdgyService, WsAccessor, HttpAccessor},
};

// Define your state type
#[derive(Debug, Default)]
struct AppState {
    user_count: u32,
    messages: Vec<String>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Create service with state
    let service: EdgyService<AppState> = EdgyService::builder_with_state(
        "0.0.0.0:8080",
        AppState::default(),
    )
    .workers(4)
    .build()
    .await?;
    
    // In handlers, access state via accessor
    // ...
    
    service.run().await
}

// Access state in WebSocket handler
async fn my_handler(accessor: WsAccessor<AppState>, data: String) -> String {
    // Read state
    let state = accessor.borrow().await;
    println!("User count: {}", state.user_count);
    drop(state);
    
    // Mutate state
    let mut state = accessor.borrow_mut().await;
    state.messages.push(data);
    format!("Message #{} received", state.messages.len())
}

// on_open receives HttpAccessor (before WebSocket upgrade)
async fn on_open(accessor: HttpAccessor<AppState>) {
    let mut state = accessor.borrow_mut().await;
    state.user_count += 1;
}
```

### Client with State

```rust
use edgy_s::{
    Binding, WsAsyncFn,
    client::{EdgyClient, WsAccessor},
};

#[derive(Debug, Default)]
struct ClientState {
    request_count: u32,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let client: EdgyClient<ClientState> = EdgyClient::builder_with_state(
        "ws://localhost:8080",
        ClientState::default(),
    )?
    .workers(2)
    .build()?;
    
    // ...
    
    client.run().await
}

async fn handler(accessor: WsAccessor<ClientState>, msg: String) -> String {
    let state = accessor.borrow().await;
    println!("Requests sent: {}", state.request_count);
    "ok".into()
}
```

### State Access Methods

| Method | Description |
|--------|-------------|
| `borrow().await` | Get read guard (`Ref<S>`) - multiple concurrent readers |
| `borrow_mut().await` | Get write guard (`RefMut<S>`) - exclusive access |

## Cross-Protocol Communication

HTTP handlers can communicate with WebSocket connections using `find_ws_conn()`:

```rust
use edgy_s::{
    Binding, HttpServerAsyncFn, WsAsyncFn,
    server::{EdgyService, HttpAccessor, WsAccessor, WsCaller},
};

// HTTP endpoint that broadcasts to WebSocket connections
async fn broadcast_to_websocket(accessor: HttpAccessor<()>, body: String) -> String {
    // Find WebSocket connection to /chat path
    if let Some(ws_conn) = accessor.find_ws_conn(chat_handler, |_acc| true).await {
        // Send message to WebSocket client
        let _: String = (body.clone(),)
            .call_remotely(&ws_conn)
            .await
            .unwrap_or_else(|_| "Failed".into());
        format!("Broadcasted: {}", body)
    } else {
        "No WebSocket connections".to_string()
    }
}

async fn chat_handler(_accessor: WsAccessor<()>, msg: String) -> String {
    println!("Received: {}", msg);
    "ack".into()
}
```

## Feature Flags

| Flag | Description |
|------|-------------|
| `client` | Enable client functionality (WebSocket + HTTP) |
| `server` | Enable server functionality (WebSocket + HTTP) |
| `req_id_u8` | Use u8 for request IDs (default) |
| `req_id_u16` | Use u16 for request IDs |
| `req_id_u32` | Use u32 for request IDs |
| `req_id_u64` | Use u64 for request IDs |

## License

Licensed under [Apache-2.0](./LICENSE).
