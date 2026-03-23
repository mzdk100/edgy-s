use {
    super::{
        super::{
            types::{Accessor, HttpClientAsyncFn, IntoStreamingBody, StreamingBody},
            utils::{append_query_params, build_uri, get_path},
        },
        HttpAccessor, HttpCall, IoError, IoResult, RequestConn, ResponseConn,
    },
    hyper::{Method, Uri},
    std::{any::type_name, collections::HashMap, sync::LazyLock},
    tokio::{
        sync::{
            Mutex, mpsc::Sender as MpscSender, oneshot::channel as oneshot_channel,
            watch::channel as watch_channel,
        },
        time::{Duration, sleep},
    },
    tracing::info,
};

/// Configuration for HTTP request retry behavior
#[derive(Clone)]
pub(super) struct HttpBindingConfig {
    pub sender: MpscSender<HttpCall>,
    pub base_url: Uri,
    pub max_retries: usize,
    pub retry_interval: Duration,
}

pub(super) static HTTP_BINDING_SENDERS: LazyLock<Mutex<HashMap<String, HttpBindingConfig>>> =
    LazyLock::new(Default::default);

/// HTTP GET request trait
pub trait HttpGet<Body, Acc>
where
    Body: From<StreamingBody>,
{
    fn get<F>(self, f: F) -> impl Future<Output = IoResult<(Body, Accessor<Acc>)>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl<Body> HttpGet<Body, ResponseConn> for ()
where
    Body: From<StreamingBody>,
{
    async fn get<F>(self, f: F) -> IoResult<(Body, HttpAccessor)>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        http_request(f, Method::GET, ()).await
    }
}

/// HTTP HEAD request trait
pub trait HttpHead<Acc> {
    fn head<F>(self, f: F) -> impl Future<Output = IoResult<Accessor<Acc>>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl HttpHead<ResponseConn> for () {
    async fn head<F>(self, f: F) -> IoResult<HttpAccessor>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        let (_, accessor) = http_request::<_, StreamingBody, F>(f, Method::HEAD, ()).await?;
        Ok(accessor)
    }
}

/// HTTP DELETE request trait
pub trait HttpDelete<Body, Acc>
where
    Body: From<StreamingBody>,
{
    fn delete<F>(self, f: F) -> impl Future<Output = IoResult<(Body, Accessor<Acc>)>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl<Body> HttpDelete<Body, ResponseConn> for ()
where
    Body: From<StreamingBody>,
{
    async fn delete<F>(self, f: F) -> IoResult<(Body, HttpAccessor)>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        http_request(f, Method::DELETE, ()).await
    }
}

/// HTTP POST request trait
pub trait HttpPost<ReqBody, ResBody, Acc>
where
    ReqBody: IntoStreamingBody,
    ResBody: From<StreamingBody>,
{
    fn post<F>(self, f: F) -> impl Future<Output = IoResult<(ResBody, Accessor<Acc>)>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl<ReqBody, ResBody> HttpPost<ReqBody, ResBody, ResponseConn> for ReqBody
where
    ReqBody: IntoStreamingBody + Clone,
    ResBody: From<StreamingBody>,
{
    async fn post<F>(self, f: F) -> IoResult<(ResBody, HttpAccessor)>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        http_request(f, Method::POST, self).await
    }
}

/// HTTP PUT request trait
pub trait HttpPut<ReqBody, ResBody, Acc>
where
    ReqBody: IntoStreamingBody,
    ResBody: From<StreamingBody>,
{
    fn put<F>(self, f: F) -> impl Future<Output = IoResult<(ResBody, Accessor<Acc>)>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl<ReqBody, ResBody> HttpPut<ReqBody, ResBody, ResponseConn> for ReqBody
where
    ReqBody: IntoStreamingBody + Clone,
    ResBody: From<StreamingBody>,
{
    async fn put<F>(self, f: F) -> IoResult<(ResBody, HttpAccessor)>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        http_request(f, Method::PUT, self).await
    }
}

/// HTTP PATCH request trait
pub trait HttpPatch<ReqBody, ResBody, Acc>
where
    ReqBody: IntoStreamingBody,
    ResBody: From<StreamingBody>,
{
    fn patch<F>(self, f: F) -> impl Future<Output = IoResult<(ResBody, Accessor<Acc>)>>
    where
        F: HttpClientAsyncFn<RequestConn>;
}

impl<ReqBody, ResBody> HttpPatch<ReqBody, ResBody, ResponseConn> for ReqBody
where
    ReqBody: IntoStreamingBody + Clone,
    ResBody: From<StreamingBody>,
{
    async fn patch<F>(self, f: F) -> IoResult<(ResBody, HttpAccessor)>
    where
        F: HttpClientAsyncFn<RequestConn>,
    {
        http_request(f, Method::PATCH, self).await
    }
}

/// Internal function to send HTTP request with retry support
async fn http_request<ReqBody, ResBody, F>(
    f: F,
    method: Method,
    body: ReqBody,
) -> IoResult<(ResBody, Accessor<ResponseConn>)>
where
    ReqBody: IntoStreamingBody + Clone,
    ResBody: From<StreamingBody>,
    F: HttpClientAsyncFn<RequestConn>,
{
    let path = get_path::<F>();
    let Some(config) = HTTP_BINDING_SENDERS.lock().await.get(&path).cloned() else {
        return Err(IoError::other(format!(
            "The `{}` path used by the current `{}` function has not been bound to the client.",
            path,
            type_name::<F>()
        )));
    };

    // Combine base_url with path
    let uri = build_uri(&config.base_url, &path, Some("http"))?;

    // Create channels for request headers and query params
    let (header_tx, header_rx) = watch_channel(Default::default());
    let (query_tx, query_rx) = watch_channel(Default::default());
    let accessor: RequestConn = (uri.clone(), header_tx, query_tx).into();

    // Call handler to set headers and query params
    f.call(accessor).await;

    // Get the headers set by handler
    let headers = header_rx.borrow().clone();

    // Get query params and append to URI
    let query_params = query_rx.borrow().clone();
    let uri = append_query_params(&uri, &query_params);
    info!("Connect to {}", uri);

    // Retry loop
    let mut last_error = None;
    for attempt in 0..config.max_retries {
        // Send HTTP request
        let (response_tx, response_rx) = oneshot_channel();
        if let Err(e) = config
            .sender
            .send((
                uri.clone(),
                method.clone(),
                headers.clone(),
                body.clone().into_streaming_body(),
                response_tx,
            ))
            .await
        {
            last_error = Some(IoError::other(e));
            if attempt < config.max_retries - 1 {
                sleep(config.retry_interval).await;
            }
            continue;
        }

        // Receive response
        match response_rx.await {
            Ok(Ok((response_body, status, response_headers))) => {
                // Build ResponseConn
                let response_conn: ResponseConn = (uri, status, response_headers).into();
                return Ok((response_body.into(), response_conn.into()));
            }
            Ok(Err(e)) => {
                last_error = Some(e);
                if attempt < config.max_retries - 1 {
                    sleep(config.retry_interval).await;
                }
                continue;
            }
            Err(e) => {
                last_error = Some(IoError::other(e));
                if attempt < config.max_retries - 1 {
                    sleep(config.retry_interval).await;
                }
                continue;
            }
        }
    }

    // All retries failed
    Err(last_error.unwrap_or_else(|| IoError::other("HTTP request failed after all retries")))
}
