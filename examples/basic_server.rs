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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    let service = EdgyService::builder("0.0.0.0:80")
        .workers(1)
        .build()
        .await?;
    let bd_api_add = api_add
        .bind(&service)
        .await?
        .on_open(api_add_open)
        .await
        .on_close(api_add_close)
        .await;
    let bd_index = index.bind_by_path_as_response(&service, "/").await?;
    let bd_index2 = index.bind_as_response(&service).await?;
    service.run().await?;
    bd_api_add.unbind().await?;
    bd_index.unbind().await?;
    bd_index2.unbind().await?;

    Ok(())
}

async fn index(accessor: HttpAccessor, body: String) -> String {
    let name = accessor.get_argument("name").unwrap_or_default();
    let _ = accessor.set_header("Cookie", "test=1");
    format!("<html><body>Hello {}, world, {}!</body></html>", name, body)
}

async fn api_add_open(accessor: HttpAccessor) {
    println!("WebSocket opened from: {}", accessor.get_addr());
}

async fn api_add_close(accessor: WsAccessor) {
    println!("WebSocket closed from: {}", accessor.get_addr());
}

async fn api_add(accessor: WsAccessor, a: i32, b: i32) -> i32 {
    println!("{} + {}, call from: {}", a, b, accessor.get_addr());
    tokio::spawn(async move {
        let client_return: i32 = (5, 5).call_remotely(&accessor).await.unwrap();
        println!("5 + 5 = {}, from: client", client_return);

        println!("Other conns:");
        for acc in accessor.get_other_conns().await {
            println!("{}", acc.get_addr());
        }
    });

    a + b
}
