use {
    edgy_s::{AsyncFun, ClientAccessor, EdgyClient, ServiceCaller},
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

    let client = EdgyClient::new("ws://localhost", 1)?;
    api_add.bind(&client).await?;

    tokio::spawn(async {
        let ret = (1, 2).call_remotely(api_add).await?;
        println!("1 + 2 = {}, from: server", ret);

        Ok::<_, anyhow::Error>(())
    });
    let client = client.run().await?;
    api_add.unbind(&client).await?;

    Ok(())
}

async fn api_add(accessor: ClientAccessor, a: i32, b: i32) -> i32 {
    println!("{} + {}, call from: {}", a, b, accessor.as_str());
    a + b
}
