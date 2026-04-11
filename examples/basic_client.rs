use {
    edgy_s::{
        Binding, HttpClientAsyncFn, WsAsyncFn,
        client::{EdgyClient, HttpPost, RequestAccessor, WsAccessor, WsCaller},
    },
    tracing_subscriber::{
        Layer, filter::LevelFilter, fmt::layer, layer::SubscriberExt, registry,
        util::SubscriberInitExt,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    let client = EdgyClient::builder("ws://localhost")?
        .workers(1)
        .max_retries(5)
        .build()?;
    let bd_api_add = api_add
        .bind(&client)
        .await?
        .on_open(api_add_open)
        .await
        .on_close(api_add_close)
        .await;
    let bd_index = index.bind_as_request(&client).await?;

    tokio::spawn(async {
        let (res, accessor): (String, _) = "how are you".post(index).await.unwrap();
        println!("index ({}): {}", accessor.status(), res);

        let ret = (1, 2).call_remotely(api_add).await?;
        println!("1 + 2 = {}, from: server", ret);
        Ok::<_, anyhow::Error>(())
    });

    client.run().await?;
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;

    Ok(())
}

async fn index(accessor: RequestAccessor) {
    accessor
        .set_header("User-Agent", env!("CARGO_PKG_NAME"))
        .unwrap();
    accessor.set_argument("name", "SmileSky");
}

async fn api_add_open(accessor: WsAccessor) {
    println!(
        "WebSocket opened from: {} ({})",
        accessor.path(),
        accessor.status()
    );
}

async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    println!("{} + {}, call from: {}", a, b, accessor.path());
    a + b
}

async fn api_add_close(accessor: WsAccessor) {
    println!("WebSocket closed from: {}", accessor.path());
}
