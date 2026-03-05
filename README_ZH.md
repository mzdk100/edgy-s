# edgy-s

[English](./README.md)

一个极简的 WebSocket 双向 RPC 框架，用优雅的函数式路由构建复杂的微服务应用。

## 特性

- **极简 API** - 一行代码将函数绑定为路由
- **双向 RPC** - 客户端和服务端都可以主动发起远程调用
- **类型安全** - 基于 serde 的完整类型安全序列化
- **自动路径推导** - 从函数名自动生成路由路径
- **零样板代码** - 无宏、无复杂配置
- **特性开关** - 按需引入客户端/服务端功能

## 快速开始

### 添加依赖

```toml
[dependencies]
edgy-s = { version = "0.1", features = ["server", "client"] }
```

### 服务端示例

```rust
use edgy_s::{AsyncFun, ClientCaller, EdgyService, ServiceAccessor};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = EdgyService::new("0.0.0.0:8080", 4).await?;
    
    // 绑定函数作为路由 - 就这么简单！
    api_add.bind(&service).await?;
    
    service.run().await?;
    Ok(())
}

// 定义你的 API 函数
async fn api_add(accessor: ServiceAccessor, a: i32, b: i32) -> i32 {
    // 服务端也可以调用客户端的方法！
    tokio::spawn(async move {
        let result: i32 = (5, 5).call_remotely(&accessor).await.unwrap();
        println!("客户端计算: 5 + 5 = {}", result);
    });
    
    a + b
}
```

### 客户端示例

```rust
use edgy_s::{AsyncFun, ClientAccessor, EdgyClient, ServiceCaller};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = EdgyClient::new("ws://localhost:8080", 2)?;
    
    // 绑定以接收服务端调用
    api_add.bind(&client).await?;
    
    // 调用服务端方法
    tokio::spawn(async {
        let result = (1, 2).call_remotely(api_add).await.unwrap();
        println!("服务端计算: 1 + 2 = {}", result);
    });
    
    client.run().await?;
    Ok(())
}

// 客户端也可以处理服务端的调用
async fn api_add(accessor: ClientAccessor, a: i32, b: i32) -> i32 {
    a + b
}
```

## 请求 ID 配置

根据你的并发需求选择合适的请求 ID 宽度：

| Feature | 类型 | 最大并发请求数 |
|---------|------|---------------|
| `req_id_u8` (默认) | u8 | 256 |
| `req_id_u16` | u16 | 65,536 |
| `req_id_u32` | u32 | 约 42 亿 |
| `req_id_u64` | u64 | 几乎无限 |

```toml
[dependencies]
edgy-s = { version = "0.1", features = ["server", "client", "req_id_u32"] }
```

## 特性开关

| Flag | 描述 |
|------|------|
| `client` | 启用 WebSocket 客户端功能 |
| `server` | 启用 WebSocket 服务端功能 |
| `req_id_u8` | 使用 u8 作为请求 ID（默认） |
| `req_id_u16` | 使用 u16 作为请求 ID |
| `req_id_u32` | 使用 u32 作为请求 ID |
| `req_id_u64` | 使用 u64 作为请求 ID |

## API 设计理念

### 函数即路由

路由路径自动从函数名推导：

```rust
async fn my_api_handler(...) { ... }
// 路由: /my/api/handler
```

### 绑定即注册

```rust
my_handler.bind(&service).await?;  // 注册并连接
my_handler.unbind(&service).await?; // 注销并断开
```

### 参数即调用

```rust
// 用参数调用远程函数
(参数).call_remotely(handler).await
```

## 许可证

基于 [Apache-2.0](./LICENSE) 许可证发布。
