use {
    async_stream::stream,
    edgy_s::{
        Binding, HttpServerAsyncFn,
        server::{EdgyService, HttpAccessor},
    },
    futures_util::{Stream, StreamExt},
    std::{io::Result as IoResult, pin::Pin},
    tokio::time::{Duration, sleep},
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
    let bd_countdown = countdown.bind_as_response(&service).await?;
    let bd_echo_stream = echo_stream.bind_as_response(&service).await?;
    service.run().await?;
    bd_countdown.unbind().await?;
    bd_echo_stream.unbind().await?;

    Ok(())
}

async fn countdown(_accessor: HttpAccessor, _body: String) -> Pin<Box<impl Stream<Item = String>>> {
    let from = _accessor
        .get_argument("from")
        .and_then(|i| i.parse().ok())
        .unwrap_or(10u8);

    Box::pin(stream! {
        yield format!("<p>Countdown from {}</p><br>", from);
        for i in (0..from).rev() {
            sleep(Duration::from_secs(1)).await;
            yield format!("<p>{}</p>", i);
        }
    })
}

/// Stream echo: receives body as a stream and echoes each chunk back.
/// Demonstrates streaming body reception and response.
async fn echo_stream<S>(
    _accessor: HttpAccessor,
    mut body: Pin<Box<S>>,
) -> Pin<Box<impl Stream<Item = String>>>
where
    S: Stream<Item = IoResult<String>> + ?Sized,
{
    Box::pin(stream! {
        yield "<pre>".into();
        while let Some(Ok(chunk)) = body.next().await {
            // Echo each chunk as it arrives
            yield format!("[{} bytes]: {}", chunk.len(), chunk);
        }
        yield "</pre>".into();
    })
}
