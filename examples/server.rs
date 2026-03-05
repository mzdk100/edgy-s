use {
    edgy_s::{AsyncFun, ClientCaller, EdgyService, ServiceAccessor},
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

    let service = EdgyService::new("0.0.0.0:80", 1).await?;
    api_add.bind(&service).await?;
    let service = service.run().await?;
    api_add.unbind(&service).await?;

    Ok(())
}

async fn api_add(accessor: ServiceAccessor, a: i32, b: i32) -> i32 {
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
