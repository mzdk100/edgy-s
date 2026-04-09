use {
    super::{EdgyService, IoResult, State},
    tokio::{
        net::ToSocketAddrs,
        runtime::Builder,
        sync::{RwLock, mpsc::channel as mpsc_channel},
    },
};

/// Default configuration values
const DEFAULT_NUM_WORKERS: usize = 4;

/// Builder for creating `EdgyService` with custom configuration.
///
/// # Example
/// ```ignore
/// use edgy_s::server::EdgyService;
///
/// let service = EdgyService::builder("0.0.0.0:80")
///     .workers(2)
///     .build()
///     .await?;
/// ```
pub struct EdgyServiceBuilder<Addr, S = ()> {
    bind_addr: Addr,
    num_workers: usize,
    state: State<S>,
}

impl<Addr, S> EdgyServiceBuilder<Addr, S>
where
    Addr: ToSocketAddrs,
{
    /// Sets the number of worker threads for the async runtime.
    pub fn workers(mut self, num: usize) -> Self {
        self.num_workers = num;
        self
    }

    /// Builds the `EdgyService` with the configured settings.
    pub async fn build(self) -> IoResult<EdgyService<S>>
    where
        S: Send + Sync + 'static,
    {
        let rt = Builder::new_multi_thread()
            .worker_threads(self.num_workers)
            .enable_all()
            .build()?;

        let (command_tx, command_rx) = mpsc_channel(2);
        let worker_task = rt.spawn(EdgyService::<S>::worker(command_rx));

        Ok(EdgyService {
            command: command_tx,
            bind_addr: EdgyService::<S>::get_bind_addr(self.bind_addr).await?,
            rt: rt.into(),
            worker_task,
            state: self.state,
        })
    }

    pub(super) fn new(bind_addr: Addr, state: S) -> Self {
        Self {
            bind_addr,
            num_workers: DEFAULT_NUM_WORKERS,
            state: RwLock::new(state).into(),
        }
    }
}
