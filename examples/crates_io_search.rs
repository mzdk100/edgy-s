use {
    edgy_s::{
        Binding, HttpClientAsyncFn,
        client::{EdgyClient, HttpGet, RequestAccessor},
    },
    tracing_subscriber::{
        Layer, filter::LevelFilter, fmt::layer, layer::SubscriberExt, registry,
        util::SubscriberInitExt,
    },
};

fn main() -> anyhow::Result<()> {
    registry()
        .with(layer().without_time().with_filter(LevelFilter::INFO))
        .init();

    let client = EdgyClient::new("https://crates.io")?;
    let rt = client.rt().clone();

    rt.block_on(async {
        let bd_api_v1_crates = api_v1_crates.bind_as_request(&client).await?;

        let (res, accessor): (String, _) = ().get(api_v1_crates).await?;
        println!("search ({}): {}", accessor.status(), res);

        bd_api_v1_crates.unbind().await?;
        client.abort();

        Ok::<_, anyhow::Error>(())
    })
}

async fn api_v1_crates(accessor: RequestAccessor) {
    accessor
        .set_header("User-Agent", env!("CARGO_PKG_NAME"))
        .unwrap();
    accessor.set_argument("q", env!("CARGO_PKG_NAME"));
}
