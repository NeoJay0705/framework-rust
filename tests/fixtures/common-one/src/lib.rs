use std::sync::{Arc, Mutex};

static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static PRIMARY_REGISTRY: Mutex<Option<Arc<Mutex<Vec<&'static str>>>>> = Mutex::new(None);
static ALTERNATE_REGISTRY: Mutex<Option<Arc<Mutex<Vec<&'static str>>>>> = Mutex::new(None);

pub fn record(event: &'static str) {
    EVENTS.lock().expect("event lock poisoned").push(event);
}

pub fn reset_events() {
    EVENTS.lock().expect("event lock poisoned").clear();
    *PRIMARY_REGISTRY
        .lock()
        .expect("primary registry lock poisoned") = None;
    *ALTERNATE_REGISTRY
        .lock()
        .expect("alternate registry lock poisoned") = None;
}

pub fn take_events() -> Vec<&'static str> {
    std::mem::take(&mut *EVENTS.lock().expect("event lock poisoned"))
}

pub struct Config(pub usize);
pub struct Database(pub usize);
pub struct CommonLifecycle;

impl framework::Lifecycle for CommonLifecycle {
    async fn start(&self) -> framework::LifecycleResult {
        record("start.lifecycle-common");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        record("shutdown.lifecycle-common");
        Ok(())
    }
}

pub struct Registry {
    values: Arc<Mutex<Vec<&'static str>>>,
}

impl Registry {
    pub fn push(&mut self, value: &'static str) {
        self.values
            .lock()
            .expect("registry values lock poisoned")
            .push(value);
    }
}

pub fn registry_values(qualifier: &str) -> Vec<&'static str> {
    let registry = match qualifier {
        "primary" => PRIMARY_REGISTRY
            .lock()
            .expect("primary registry lock poisoned")
            .clone(),
        "alternate" => ALTERNATE_REGISTRY
            .lock()
            .expect("alternate registry lock poisoned")
            .clone(),
        _ => None,
    }
    .expect("registry provider should have stored its value");
    registry
        .lock()
        .expect("registry values lock poisoned")
        .clone()
}

pub fn registry_effective(qualifier: &str) -> Option<&'static str> {
    registry_values(qualifier).last().copied()
}

pub struct CommonRoutes;
impl framework::Register<Registry> for CommonRoutes {
    fn register(&self, target: &mut Registry) {
        target.push("common");
        record("register.common");
    }
}

pub struct UnreachableRoutes;
impl framework::Register<Registry> for UnreachableRoutes {
    fn register(&self, target: &mut Registry) {
        target.push("unreachable");
        record("register.unreachable");
    }
}

pub struct ImplicitRoutes;
impl framework::Register<Registry> for ImplicitRoutes {
    fn register(&self, target: &mut Registry) {
        target.push("implicit");
        record("register.implicit-discovered");
    }
}

#[framework::bundle(name = common_bundle)]
pub mod common {
    #[provide]
    fn config() -> crate::Config {
        crate::record("construct.config");
        crate::Config(23)
    }

    #[provide(qualifier = "primary")]
    fn primary(config: &crate::Config) -> std::sync::Arc<crate::Database> {
        crate::record("construct.primary");
        std::sync::Arc::new(crate::Database(config.0))
    }

    #[provide(qualifier = "secondary")]
    fn secondary(config: &crate::Config) -> std::sync::Arc<crate::Database> {
        crate::record("construct.unreachable-secondary");
        std::sync::Arc::new(crate::Database(config.0 + 1))
    }

    #[provide(qualifier = "primary")]
    fn primary_registry() -> crate::Registry {
        crate::record("construct.registry-primary");
        let values = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        *crate::PRIMARY_REGISTRY
            .lock()
            .expect("primary registry lock poisoned") = Some(values.clone());
        crate::Registry { values }
    }

    #[provide(qualifier = "alternate")]
    fn alternate_registry() -> crate::Registry {
        crate::record("construct.registry-alternate");
        let values = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        *crate::ALTERNATE_REGISTRY
            .lock()
            .expect("alternate registry lock poisoned") = Some(values.clone());
        crate::Registry { values }
    }

    #[provide(register(crate::Registry, qualifier = "primary"))]
    fn common_routes(#[inject("primary")] _registry: &crate::Registry) -> crate::CommonRoutes {
        crate::record("construct.common-routes");
        crate::CommonRoutes
    }

    #[provide(register(crate::Registry, qualifier = "primary"))]
    fn unreachable_routes() -> crate::UnreachableRoutes {
        crate::record("construct.unreachable-routes");
        crate::UnreachableRoutes
    }

    #[provide]
    fn implicit_routes() -> crate::ImplicitRoutes {
        crate::record("construct.implicit-routes");
        crate::ImplicitRoutes
    }

    #[provide(lifecycle)]
    fn lifecycle_component() -> crate::CommonLifecycle {
        crate::record("construct.lifecycle-common");
        crate::CommonLifecycle
    }

    #[hook(start)]
    fn observe_config(_config: &crate::Config) {
        crate::record("hook.common");
    }
}
