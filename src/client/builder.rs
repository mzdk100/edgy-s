use {
    super::{EdgyClient, IoResult, State},
    hyper::Uri,
    std::time::Duration,
    tokio::{
        runtime::Builder,
        sync::{RwLock, mpsc::channel as mpsc_channel},
    },
};

/// Default configuration values
const DEFAULT_NUM_WORKERS: usize = 4;
const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_RETRY_INTERVAL_MS: u64 = 1000;

/// Builder for creating `EdgyClient` with custom configuration.
///
/// # Example
/// ```ignore
/// use edgy_s::client::EdgyClient;
///
/// let client = EdgyClient::builder("ws://localhost")?
///     .workers(2)
///     .max_retries(5)
///     .retry_interval_ms(500)
///     .build()?;
/// ```
pub struct EdgyClientBuilder<S = ()> {
    base_url: Uri,
    num_workers: usize,
    max_retries: usize,
    retry_interval: Duration,
    state: State<S>,
}

impl<S> EdgyClientBuilder<S> {
    pub(super) fn new(base_url: Uri, state: S) -> Self {
        Self {
            base_url,
            num_workers: DEFAULT_NUM_WORKERS,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_interval: Duration::from_millis(DEFAULT_RETRY_INTERVAL_MS),
            state: RwLock::new(state).into(),
        }
    }

    /// Sets the number of worker threads for the async runtime.
    pub fn workers(mut self, num: usize) -> Self {
        self.num_workers = num;
        self
    }

    /// Sets the maximum number of reconnection attempts for WebSocket connections.
    pub fn max_retries(mut self, num: usize) -> Self {
        self.max_retries = num;
        self
    }

    /// Sets the retry interval in milliseconds between reconnection attempts.
    pub fn retry_interval_ms(mut self, ms: u64) -> Self {
        self.retry_interval = Duration::from_millis(ms);
        self
    }

    /// Sets the retry interval as a Duration.
    pub fn retry_interval(mut self, duration: Duration) -> Self {
        self.retry_interval = duration;
        self
    }

    /// Builds the `EdgyClient` with the configured settings.
    pub fn build(self) -> IoResult<EdgyClient<S>>
    where
        S: 'static,
    {
        let rt = Builder::new_multi_thread()
            .worker_threads(self.num_workers)
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc_channel(2);
        let state = self.state.clone();
        let task = rt.spawn(EdgyClient::<S>::worker(rx));

        Ok(EdgyClient {
            base_url: self.base_url,
            rt: rt.into(),
            command: tx,
            task: Some(task),
            max_retries: self.max_retries,
            retry_interval: self.retry_interval,
            state,
        })
    }
}
