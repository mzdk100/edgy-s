# edgy-s

[English](./README.md)

一个极简的 WebSocket/HTTP 双向 RPC 框架，用优雅的函数式路由构建复杂的微服务应用。

## 特性

- **极简 API** - 一行代码将函数绑定为路由
- **双向 RPC** - 客户端和服务端都可以主动发起远程调用
- **HTTP 支持** - 完整的 HTTP 请求/响应处理，支持流式传输
- **类型安全** - 严格的类型约束和基于 serde 的完整序列化
- **自动路径推导** - 从函数名自动生成路由路径
- **零样板代码** - 无宏、无复杂配置
- **特性开关** - 按需引入客户端/服务端功能
- **自动重连** - WebSocket 客户端支持可配置的重试逻辑
- **Builder 模式** - 灵活的客户端/服务端配置
- **共享状态** - 内置基于 `Arc<RwLock<S>>` 的状态管理，支持并发访问
- **多种序列化后端** - 支持 postcard（默认）、CBOR 和 JSON

## 安装

```toml
[dependencies]
edgy-s = { version = "1.3", features = ["server", "client"] }
```

## 快速开始

### 服务端示例

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
    // 使用 Builder 模式创建服务
    let service = EdgyService::builder("0.0.0.0:8080")
        .workers(4)
        .build()
        .await?;
    
    // 绑定 WebSocket 路由，支持生命周期钩子
    let bd_api_add = api_add
        .bind(&service)
        .await?
        .on_open(on_open)
        .await
        .on_close(on_close)
        .await;
    
    // 绑定 HTTP 路由
    let bd_index = index.bind_as_response(&service).await?;
    let bd_stream = countdown.bind_as_response(&service).await?;
    
    service.run().await?;
    
    // 清理
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;
    bd_stream.unbind().await?;
    
    Ok(())
}

// WebSocket 处理器 - 双向 RPC
async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    // 服务端也可以调用客户端的方法！
    tokio::spawn(async move {
        let result: i32 = (5, 5).call_remotely(&accessor).await.unwrap();
        println!("客户端计算: 5 + 5 = {}", result);
    });
    a + b
}

async fn on_open(accessor: HttpAccessor) {
    println!("WebSocket 连接来自: {}", accessor.get_addr());
}

async fn on_close(accessor: WsAccessor) {
    println!("WebSocket 断开来自: {}", accessor.get_addr());
}

// HTTP 处理器 - 简单响应
async fn index(accessor: HttpAccessor, body: String) -> String {
    let name = accessor.get_argument("name").unwrap_or_default();
    accessor.set_header("Content-Type", "text/html").unwrap();
    format!("<html><body>Hello {}, {}!</body></html>", name, body)
}

// HTTP 处理器 - 流式响应
async fn countdown(accessor: HttpAccessor, _body: String) -> Pin<Box<impl Stream<Item = String>>> {
    let from = accessor.get_argument("from")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10u8);
    
    Box::pin(stream! {
        yield format!("<p>从 {} 倒计时</p>", from);
        for i in (0..from).rev() {
            sleep(Duration::from_secs(1)).await;
            yield format!("<p>{}</p>", i);
        }
    })
}
```

### 客户端示例

```rust
use edgy_s::{
    Binding, HttpClientAsyncFn, WsAsyncFn,
    client::{EdgyClient, HttpPost, RequestAccessor, WsAccessor, WsCaller},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 使用 Builder 模式创建客户端
    let client = EdgyClient::builder("ws://localhost:8080")?
        .workers(2)
        .max_retries(5)
        .retry_interval_ms(1000)
        .build()?;
    
    // 绑定 WebSocket 路由
    let bd_api_add = api_add
        .bind(&client)
        .await?
        .on_open(on_open)
        .await
        .on_close(on_close)
        .await;
    
    // 绑定 HTTP 路由
    let bd_index = index.bind_as_request(&client).await?;
    
    // 发送 HTTP POST 请求
    tokio::spawn(async {
        let (response, accessor): (String, _) = "请求体"
            .post(index)
            .await
            .unwrap();
        println!("状态码: {}, 响应: {}", accessor.status(), response);
    });
    
    // 通过 WebSocket RPC 调用服务端
    tokio::spawn(async {
        let result: i32 = (1, 2).call_remotely(api_add).await.unwrap();
        println!("服务端计算: 1 + 2 = {}", result);
    });
    
    client.run().await?;
    
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;
    
    Ok(())
}

// WebSocket 处理器 - 接收服务端调用
async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    println!("服务端请求: {} + {}", a, b);
    a + b
}

// HTTP 请求配置器
async fn index(accessor: RequestAccessor) {
    accessor.set_header("User-Agent", "edgy-s-client").unwrap();
    accessor.set_argument("name", "world");
}

async fn on_open(accessor: WsAccessor) {
    println!("已连接到: {}", accessor.path());
}

async fn on_close(accessor: WsAccessor) {
    println!("已断开: {}", accessor.path());
}
```

## API 参考

### 客户端配置

```rust
let client = EdgyClient::builder("ws://localhost:8080")?
    .workers(4)              // 异步工作线程数
    .max_retries(5)          // WebSocket 最大重连次数
    .retry_interval_ms(1000) // 重试间隔毫秒
    .retry_interval(Duration::from_secs(1)) // 或使用 Duration
    .build()?;
```

### 服务端配置

```rust
let service = EdgyService::builder("0.0.0.0:8080")
    .workers(4)
    .build()
    .await?;
```

### HTTP 方法

```rust
// GET 请求
let (body, accessor): (String, _) = ().get(handler).await?;

// POST 请求
let (body, accessor): (String, _) = "body".post(handler).await?;

// PUT 请求
let (body, accessor): (String, _) = "body".put(handler).await?;

// PATCH 请求
let (body, accessor): (String, _) = "body".patch(handler).await?;

// DELETE 请求
let (body, accessor): (String, _) = ().delete(handler).await?;

// HEAD 请求
let accessor: HttpAccessor = ().head(handler).await?;
```

### Accessor 方法

#### 服务端 (WsAccessor / HttpAccessor)

| 方法 | 描述 |
|------|------|
| `get_addr()` | 获取客户端 socket 地址 |
| `get_argument(name)` | 获取 URL 查询参数（已解码） |
| `get_arguments(name)` | 获取参数的所有值 |
| `get_all_arguments()` | 获取所有查询参数为 HashMap |
| `get_header(name)` | 获取请求头 |
| `get_headers()` | 获取所有请求头 |
| `set_header(name, value)` | 设置响应头 |
| `add_header(name, value)` | 追加响应头 |
| `set_status(status)` | 设置 HTTP 状态码（仅 HttpAccessor） |
| `get_other_conns()` | 获取同路径的其他连接（仅 WsAccessor） |
| `find_conn(target, predicate)` | 查找连接到特定路径且满足条件的 WebSocket 连接（仅 WsAccessor） |
| `find_ws_conn(target, predicate)` | 查找连接到特定路径且满足条件的 WebSocket 连接（仅 HttpAccessor） |

#### 客户端 (WsAccessor / RequestAccessor)

| 方法 | 描述 |
|------|------|
| `path()` | 获取请求路径 |
| `status()` | 获取 HTTP 状态码（连接后的 WsAccessor） |
| `get_header(name)` | 获取响应头 |
| `get_headers()` | 获取所有响应头 |
| `set_header(name, value)` | 设置请求头（RequestAccessor） |
| `add_header(name, value)` | 追加请求头（RequestAccessor） |
| `set_argument(name, value)` | 设置 URL 查询参数（RequestAccessor） |

## 1.0 版本破坏性变更

### API 重命名

| 0.x | 1.0 |
|-----|-----|
| `AsyncFun` | `WsAsyncFn` |
| `ServiceAccessor` | `WsAccessor`（服务端） |
| `ClientAccessor` | `WsAccessor`（客户端） |
| `ClientCaller` / `ServiceCaller` | `WsCaller` |

### 构造函数变更

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

### 新增 HTTP 支持

1.0 版本为客户端和服务端添加了完整的 HTTP 支持：

- 服务端：`bind_as_response()`、`bind_by_path_as_response()`
- 客户端：`bind_as_request()`、`bind_by_path_as_request()`
- HTTP 方法：`get()`、`post()`、`put()`、`patch()`、`delete()`、`head()`
- 流式请求/响应体

### 生命周期钩子

```rust
// 0.x - 无生命周期钩子

// 1.0 - 链式调用生命周期处理器
binding
    .on_open(handler)
    .await
    .on_close(handler)
    .await
```

## 请求 ID 配置

根据你的并发需求选择合适的请求 ID 宽度：

| Feature | 类型 | 最大并发请求数 |
|---------|------|---------------|
| `req_id_u8`（默认） | u8 | 256 |
| `req_id_u16` | u16 | 65,536 |
| `req_id_u32` | u32 | 约 42 亿 |
| `req_id_u64` | u64 | 几乎无限 |

```toml
[dependencies]
edgy-s = { version = "1.3", features = ["server", "client", "req_id_u32"] }
```

## 序列化后端配置

根据需求选择一种序列化后端。优先级为：`postcard` > `cbor4` > `serde_json`。

| Feature | 库 | 格式 | 大小 | 适用场景 |
|---------|---------|--------|------|----------|
| `postcard`（默认） | postcard | 自定义二进制 | **最紧凑** | 最高效率、嵌入式系统 |
| `cbor4` | cbor4ii | CBOR | 紧凑 | 支持更多数据类型、标准格式 |
| `serde_json` | serde_json | JSON | 较大 | 人类可读、调试、Web API |

```toml
# 默认：postcard（最紧凑）
[dependencies]
edgy-s = { version = "1.3", features = ["server", "client"] }

# 使用 CBOR 标准二进制格式
[dependencies]
edgy-s = { version = "1.3", features = ["server", "client", "cbor4"] }

# 使用 JSON 人类可读格式
[dependencies]
edgy-s = { version = "1.3", features = ["server", "client", "serde_json"] }
```

**注意**：如果启用了多个序列化特性，将使用优先级最高的（postcard > cbor4 > serde_json）。必须至少启用一种序列化后端。

## 共享状态管理

`EdgyService` 和 `EdgyClient` 支持共享状态，用于管理跨连接的应用数据。

### 带状态的服务端

```rust
use edgy_s::{
    Binding, WsAsyncFn,
    server::{EdgyService, WsAccessor, HttpAccessor},
};

// 定义你的状态类型
#[derive(Debug, Default)]
struct AppState {
    user_count: u32,
    messages: Vec<String>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // 创建带状态的服务
    let service: EdgyService<AppState> = EdgyService::builder_with_state(
        "0.0.0.0:8080",
        AppState::default(),
    )
    .workers(4)
    .build()
    .await?;
    
    // 在处理器中通过 accessor 访问状态
    // ...
    
    service.run().await
}

// 在 WebSocket 处理器中访问状态
async fn my_handler(accessor: WsAccessor<AppState>, data: String) -> String {
    // 读取状态
    let state = accessor.borrow().await;
    println!("用户数: {}", state.user_count);
    drop(state);
    
    // 修改状态
    let mut state = accessor.borrow_mut().await;
    state.messages.push(data);
    format!("已收到第 {} 条消息", state.messages.len())
}

// on_open 接收 HttpAccessor（WebSocket 升级前）
async fn on_open(accessor: HttpAccessor<AppState>) {
    let mut state = accessor.borrow_mut().await;
    state.user_count += 1;
}
```

### 带状态的客户端

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
    println!("已发送请求: {}", state.request_count);
    "ok".into()
}
```

### 状态访问方法

| 方法 | 描述 |
|------|------|
| `borrow().await` | 获取读锁守卫（`Ref<S>`）- 支持多个并发读取 |
| `borrow_mut().await` | 获取写锁守卫（`RefMut<S>`）- 独占访问 |

## 跨协议通信

HTTP 处理器可以使用 `find_ws_conn()` 与 WebSocket 连接通信：

```rust
use edgy_s::{
    Binding, HttpServerAsyncFn, WsAsyncFn,
    server::{EdgyService, HttpAccessor, WsAccessor, WsCaller},
};

// 广播消息到 WebSocket 连接的 HTTP 端点
async fn broadcast_to_websocket(accessor: HttpAccessor<()>, body: String) -> String {
    // 查找 /chat 路径的 WebSocket 连接
    if let Some(ws_conn) = accessor.find_ws_conn(chat_handler, |_acc| true).await {
        // 向 WebSocket 客户端发送消息
        let _: String = (body.clone(),)
            .call_remotely(&ws_conn)
            .await
            .unwrap_or_else(|_| "发送失败".into());
        format!("已广播: {}", body)
    } else {
        "未找到 WebSocket 连接".to_string()
    }
}

async fn chat_handler(_accessor: WsAccessor<()>, msg: String) -> String {
    println!("收到: {}", msg);
    "ack".into()
}
```

## 特性开关

| Flag | 描述 |
|------|------|
| `client` | 启用客户端功能（WebSocket + HTTP） |
| `server` | 启用服务端功能（WebSocket + HTTP） |
| `req_id_u8` | 使用 u8 作为请求 ID（默认） |
| `req_id_u16` | 使用 u16 作为请求 ID |
| `req_id_u32` | 使用 u32 作为请求 ID |
| `req_id_u64` | 使用 u64 作为请求 ID |

## 许可证

基于 [Apache-2.0](./LICENSE) 许可证发布。
