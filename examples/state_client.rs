use {
    edgy_s::{
        Binding, WsAsyncFn,
        client::{EdgyClient, WsAccessor, WsCaller},
    },
    std::sync::atomic::{AtomicU32, Ordering},
    tokio::time::{Duration, sleep},
    tracing_subscriber::{
        Layer, filter::LevelFilter, fmt::layer, layer::SubscriberExt, registry,
        util::SubscriberInitExt,
    },
};

/// Client state to track message statistics
#[derive(Debug, Default)]
struct ClientState {
    sent_count: AtomicU32,
    received_count: AtomicU32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    // Create client with shared state
    let client: EdgyClient<ClientState> =
        EdgyClient::builder_with_state("ws://localhost", ClientState::default())?
            .workers(1)
            .max_retries(5)
            .build()?;

    // Bind chat handler - this connects to /chat and handles messages from server
    let bd_chat = chat
        .bind(&client)
        .await?
        .on_open(chat_open)
        .await
        .on_close(chat_close)
        .await;

    // Spawn a task to send periodic messages to server
    tokio::spawn(async {
        let mut counter = 0u32;
        loop {
            sleep(Duration::from_secs(2)).await;
            counter += 1;
            let msg = format!("Hello #{}", counter);

            // Send message to server via 'chat' route
            // The function name determines the route path
            let result: String = (msg.clone(),)
                .call_remotely(chat)
                .await
                .unwrap_or_else(|_| "Failed".into());
            println!("Server response: {}", result);
        }
    });

    client.run().await?;
    bd_chat.unbind().await?;

    Ok(())
}

/// Handle messages from server (called when server sends a request)
async fn chat(accessor: WsAccessor<ClientState>, msg: String) -> String {
    // Access shared state
    let state = accessor.borrow().await;
    state.received_count.fetch_add(1, Ordering::SeqCst);
    let received = state.received_count.load(Ordering::SeqCst);
    let sent = state.sent_count.load(Ordering::SeqCst);
    drop(state);

    println!(
        "[From server] {}: (sent: {}, received: {})",
        msg, sent, received
    );
    "ok".into()
}

async fn chat_open(accessor: WsAccessor<ClientState>) {
    println!("Connected to: {} ({})", accessor.path(), accessor.status());
}

async fn chat_close(accessor: WsAccessor<ClientState>) {
    println!("Disconnected from: {}", accessor.path());
}
