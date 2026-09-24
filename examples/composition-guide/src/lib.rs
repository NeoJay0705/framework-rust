//! Shared std-only execution support for the executable composition guide.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static HOOK_COMPLETION: Mutex<Option<mpsc::Sender<()>>> = Mutex::new(None);

pub fn record(event: impl Into<String>) {
    TRACE
        .lock()
        .expect("trace lock poisoned")
        .push(event.into());
}

pub fn take_trace() -> Vec<String> {
    std::mem::take(&mut *TRACE.lock().expect("trace lock poisoned"))
}

pub fn print_trace() {
    for event in take_trace() {
        println!("{event}");
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "timed out waiting for the example future to wake"
                );
                thread::park_timeout(remaining);
            }
        }
    }
}

#[derive(Default)]
struct StopState {
    stopped: bool,
    waker: Option<Waker>,
}

pub struct StopSender(Arc<Mutex<StopState>>);

pub struct StopFuture(Arc<Mutex<StopState>>);

impl StopSender {
    pub fn stop(self) {
        let waker = {
            let mut state = self.0.lock().expect("stop state lock poisoned");
            state.stopped = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl Future for StopFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.lock().expect("stop state lock poisoned");
        if state.stopped {
            Poll::Ready(())
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

pub fn stop_signal() -> (StopSender, StopFuture) {
    let state = Arc::new(Mutex::new(StopState::default()));
    (StopSender(Arc::clone(&state)), StopFuture(state))
}

pub fn spawn_stop_after_hook(stop: StopSender) -> thread::JoinHandle<()> {
    let (sender, receiver) = mpsc::channel();
    *HOOK_COMPLETION
        .lock()
        .expect("hook completion lock poisoned") = Some(sender);
    thread::spawn(move || {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("timed out waiting for an application hook before signaling stop");
        record("stop.signal");
        stop.stop();
    })
}

pub fn signal_hook_complete() {
    if let Some(sender) = HOOK_COMPLETION
        .lock()
        .expect("hook completion lock poisoned")
        .take()
    {
        let _ = sender.send(());
    }
}

pub struct Config {
    pub label: &'static str,
}

pub struct Metrics;

pub struct Database {
    pub config_label: &'static str,
}

pub struct ServerBackend {
    pub database: Arc<Database>,
    pub config_label: &'static str,
}

pub struct Server {
    pub database: Arc<Database>,
    pub config_label: &'static str,
}

#[derive(Default)]
pub struct Router {
    routes: Vec<&'static str>,
}

impl Router {
    pub fn effective_route(&self, name: &str) -> Option<&'static str> {
        self.routes
            .iter()
            .rev()
            .find_map(|route| route.strip_prefix(name)?.strip_prefix('='))
    }
}

pub struct BaseRoutes;

pub struct AppRoutes;

impl framework::Register<Router> for BaseRoutes {
    fn register(&self, router: &mut Router) {
        router.routes.push("home=common");
        record("register.route.common");
    }
}

impl framework::Register<Router> for AppRoutes {
    fn register(&self, router: &mut Router) {
        router.routes.push("home=application");
        record("register.route.application");
    }
}

pub struct Service {
    pub name: &'static str,
}

pub struct Worker;

#[derive(Debug)]
pub struct DemoError(pub &'static str);

impl std::fmt::Display for DemoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for DemoError {}

pub struct StartupReady;
pub struct StartupFailure;
pub struct StartupNeverStarted;

impl framework::Lifecycle for StartupReady {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.ready");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.ready");
        Err(Box::new(DemoError("ready cleanup failed")))
    }
}

impl framework::Lifecycle for StartupFailure {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.failure");
        Err(Box::new(DemoError("startup failed")))
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.failure");
        Ok(())
    }
}

impl framework::Lifecycle for StartupNeverStarted {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.never-started");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.never-started");
        Ok(())
    }
}

impl framework::Lifecycle for BaseRoutes {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.base-routes");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.base-routes");
        Ok(())
    }
}

impl framework::Lifecycle for AppRoutes {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.app-routes");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.app-routes");
        Ok(())
    }
}

impl framework::Lifecycle for Metrics {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.metrics");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.metrics");
        Ok(())
    }
}

impl framework::Lifecycle for Database {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.database");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.database");
        Ok(())
    }
}

impl framework::Lifecycle for Server {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.server");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.server");
        Ok(())
    }
}

impl framework::Lifecycle for Service {
    async fn start(&self) -> framework::LifecycleResult {
        record(format!("start.lifecycle.service.{}", self.name));
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record(format!("shutdown.lifecycle.service.{}", self.name));
        Ok(())
    }
}

impl framework::Lifecycle for Worker {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle.worker");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle.worker");
        Ok(())
    }
}

#[framework::bundle(name = base_bundle)]
pub mod base_template {
    #[provide]
    fn config() -> crate::Config {
        crate::record("construct.config");
        crate::Config { label: "demo" }
    }

    #[provide]
    fn router() -> crate::Router {
        crate::record("construct.router");
        crate::Router::default()
    }

    #[provide(lifecycle, register(crate::Router))]
    fn base_routes() -> crate::BaseRoutes {
        crate::record("construct.route.common");
        crate::BaseRoutes
    }

    #[hook(start)]
    fn initialize(config: &crate::Config) {
        let _ = config.label;
        crate::record("hook.base");
    }
}

#[framework::bundle(name = metrics_bundle)]
pub mod metrics_template {
    #[provide(lifecycle)]
    fn metrics() -> crate::Metrics {
        crate::record("construct.metrics");
        crate::Metrics
    }
}

#[framework::bundle(name = data_bundle)]
pub mod data_template {
    #[provide(lifecycle)]
    fn database(config: &crate::Config) -> std::sync::Arc<crate::Database> {
        crate::record("construct.database");
        std::sync::Arc::new(crate::Database {
            config_label: config.label,
        })
    }
}

#[framework::bundle(name = service_bundle)]
pub mod service_template {
    #[provide(qualifier = "primary", lifecycle)]
    fn primary_service(_config: &crate::Config) -> std::sync::Arc<crate::Service> {
        crate::record("construct.service.primary");
        std::sync::Arc::new(crate::Service { name: "primary" })
    }

    #[provide(qualifier = "alternate", lifecycle)]
    fn alternate_service(_config: &crate::Config) -> std::sync::Arc<crate::Service> {
        crate::record("construct.service.alternate");
        std::sync::Arc::new(crate::Service { name: "alternate" })
    }

    #[provide(lifecycle)]
    fn worker() -> crate::Worker {
        crate::record("construct.worker");
        crate::Worker
    }
}
