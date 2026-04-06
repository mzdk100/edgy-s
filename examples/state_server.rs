use {
    edgy_s::{
        Binding, WsAsyncFn,
        server::{EdgyService, HttpAccessor, WsAccessor},
    },
    tracing_subscriber::{
        Layer, filter::LevelFilter, fmt::layer, layer::SubscriberExt, registry,
        util::SubscriberInitExt,
    },
};

/// Shared state for the chat server
/// Stores chat history and connected users count
#[derive(Debug, Default)]
struct ChatState {
    messages: Vec<String>,
    user_count: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    // Create server with shared state
    let service: EdgyService<ChatState> =
        EdgyService::builder_with_state("0.0.0.0:80", ChatState::default())
            .workers(1)
            .build()
            .await?;

    // Bind chat route with lifecycle hooks
    let bd_chat = chat
        .bind(&service)
        .await?
        .on_open(chat_open)
        .await
        .on_close(chat_close)
        .await;

    println!("Chat server started on ws://localhost:80/chat");
    service.run().await?;
    bd_chat.unbind().await?;

    Ok(())
}

/// Handle incoming chat messages from clients
async fn chat(accessor: WsAccessor<ChatState>, msg: String) -> String {
    // Access shared state mutably
    let mut state = accessor.borrow_mut().await;
    state.messages.push(msg.clone());
    let count = state.messages.len();
    drop(state); // Release lock early

    println!("Received message #{}: {}", count, msg);

    format!("Message #{} received", count)
}

/// Called when a WebSocket connection is opened
/// Note: on_open receives HttpAccessor (before WebSocket upgrade)
async fn chat_open(accessor: HttpAccessor<ChatState>) {
    let mut state = accessor.borrow_mut().await;
    state.user_count += 1;
    let count = state.user_count;
    drop(state);

    println!("User joined. Total users: {}", count);
    println!("From: {}", accessor.get_addr());
}

/// Called when a WebSocket connection is closed
/// Note: on_close receives WsAccessor
async fn chat_close(accessor: WsAccessor<ChatState>) {
    let mut state = accessor.borrow_mut().await;
    state.user_count = state.user_count.saturating_sub(1);
    let count = state.user_count;
    drop(state);

    println!("User left. Total users: {}", count);
    println!("From: {}", accessor.get_addr());
}
