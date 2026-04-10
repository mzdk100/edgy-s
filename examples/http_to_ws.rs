use {
    edgy_s::{
        Binding, HttpServerAsyncFn, WsAsyncFn,
        server::{EdgyService, HttpAccessor, WsAccessor, WsCaller},
    },
    tracing_subscriber::{
        Layer, filter::LevelFilter, fmt::layer, layer::SubscriberExt, registry,
        util::SubscriberInitExt,
    },
};

/// Broadcast a message to all WebSocket connections on /chat
/// This demonstrates using HttpAccessor to find and call WebSocket connections
async fn broadcast_to_websocket(accessor: HttpAccessor, body: String) -> String {
    // Find WebSocket connection to /chat path
    // The find_ws_conn method allows HTTP handlers to communicate with WebSocket connections
    if let Some(ws_conn) = accessor.find_ws_conn(chat_handler, |_acc| true).await {
        // Broadcast the message to WebSocket client
        let _: String = (body.clone(),)
            .call_remotely(&ws_conn)
            .await
            .unwrap_or_else(|_| "Failed to send".into());

        format!("Broadcasted to WebSocket: {}", body)
    } else {
        "No WebSocket connections found".to_string()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    let service = EdgyService::builder("0.0.0.0:80")
        .workers(1)
        .build()
        .await?;

    // Bind HTTP endpoint for broadcasting messages
    let bd_broadcast = broadcast_to_websocket.bind_as_response(&service).await?;

    // Bind WebSocket handler
    let bd_chat = chat_handler.bind(&service).await?;

    println!("Server started:");
    println!("  HTTP POST /broadcast - broadcasts messages to WebSocket clients");
    println!("  WebSocket /chat - receives broadcasted messages");

    service.run().await?;
    bd_broadcast.unbind().await?;
    bd_chat.unbind().await?;

    Ok(())
}

/// WebSocket handler that receives broadcasted messages
async fn chat_handler(_accessor: WsAccessor<()>, msg: String) -> String {
    println!("WebSocket received: {}", msg);
    "ack".into()
}
