use {
    async_stream::stream,
    edgy_s::{
        Binding, HttpClientAsyncFn,
        client::{EdgyClient, HttpGet, HttpPost, RequestAccessor},
    },
    futures_util::{Stream, StreamExt},
    std::{io::Result as IoResult, pin::Pin},
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
    let bd_countdown = countdown.bind_as_request(&client).await?;
    let bd_echo_stream = echo_stream.bind_as_request(&client).await?;

    tokio::spawn(async {
        let (mut stream, accessor): (
            Pin<Box<dyn Stream<Item = IoResult<String>> + Send + Sync>>,
            _,
        ) = ().get(countdown).await?;
        println!("countdown({}): stream receiving", accessor.status());
        while let Some(msg) = stream.next().await {
            println!("{}", msg?);
        }
        println!("countdown: stream finished");

        Ok::<_, anyhow::Error>(())
    });

    tokio::spawn(async {
        let request_stream = stream! {
            yield "hello".to_string();
            yield "world".into();
            yield ",".into();
            yield "how".into();
            yield "are".into();
            yield "you".into();
            yield "!".into();
        };

        let (mut response_stream, accessor): (
            Pin<Box<dyn Stream<Item = IoResult<String>> + Send + Sync>>,
            _,
        ) = Box::pin(request_stream).post(echo_stream).await?;
        println!("echo/stream({}): stream receiving", accessor.status());
        while let Some(msg) = response_stream.next().await {
            println!("{}", msg?);
        }
        println!("echo/stream: stream finished");

        Ok::<_, anyhow::Error>(())
    });

    client.run().await?;
    bd_countdown.unbind().await?;
    bd_echo_stream.unbind().await?;

    Ok(())
}

async fn countdown(accessor: RequestAccessor) {
    accessor.set_argument("from", "10");
}

async fn echo_stream(_accessor: RequestAccessor) {}
