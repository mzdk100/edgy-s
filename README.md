# edgy-s

[中文文档](./README_ZH.md)

A minimalist WebSocket bidirectional RPC framework for building complex microservice applications with elegant, function-based routing.

## Features

- **Minimalist API** - Bind functions as routes with a single call
- **Bidirectional RPC** - Both client and server can initiate remote calls
- **Type Safe** - Full type safety with serde serialization
- **Automatic Path Derivation** - Routes are auto-generated from function names
- **Zero Boilerplate** - No macros, no complex configuration
- **Feature Flags** - Include only what you need (client/server)

## Quick Start

### Add Dependency

```toml
[dependencies]
edgy-s = { version = "0.1", features = ["server", "client"] }
```

### Server Example

```rust
use edgy_s::{AsyncFun, ClientCaller, EdgyService, ServiceAccessor};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = EdgyService::new("0.0.0.0:8080", 4).await?;
    
    // Bind function as a route - that's it!
    api_add.bind(&service).await?;
    
    service.run().await?;
    Ok(())
}

// Define your API function
async fn api_add(accessor: ServiceAccessor, a: i32, b: i32) -> i32 {
    // Server can also call client methods!
    tokio::spawn(async move {
        let result: i32 = (5, 5).call_remotely(&accessor).await.unwrap();
        println!("Client computed: 5 + 5 = {}", result);
    });
    
    a + b
}
```

### Client Example

```rust
use edgy_s::{AsyncFun, ClientAccessor, EdgyClient, ServiceCaller};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = EdgyClient::new("ws://localhost:8080", 2)?;
    
    // Bind to receive server calls
    api_add.bind(&client).await?;
    
    // Call server method
    tokio::spawn(async {
        let result = (1, 2).call_remotely(api_add).await.unwrap();
        println!("Server computed: 1 + 2 = {}", result);
    });
    
    client.run().await?;
    Ok(())
}

// Client can also handle server calls
async fn api_add(accessor: ClientAccessor, a: i32, b: i32) -> i32 {
    a + b
}
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
edgy-s = { version = "0.1", features = ["server", "client", "req_id_u32"] }
```

## Feature Flags

| Flag | Description |
|------|-------------|
| `client` | Enable WebSocket client functionality |
| `server` | Enable WebSocket server functionality |
| `req_id_u8` | Use u8 for request IDs (default) |
| `req_id_u16` | Use u16 for request IDs |
| `req_id_u32` | Use u32 for request IDs |
| `req_id_u64` | Use u64 for request IDs |

## API Design Philosophy

### Function as Route

Routes are automatically derived from function names:

```rust
async fn my_api_handler(...) { ... }
// Route: /my/api/handler
```

### Bind is Register

```rust
my_handler.bind(&service).await?;  // Registers and connects
my_handler.unbind(&service).await?; // Unregisters and disconnects
```

### Arguments as Call

```rust
// Call remote with arguments
(args).call_remotely(handler).await
```

## License

Licensed under [Apache-2.0](./LICENSE).
