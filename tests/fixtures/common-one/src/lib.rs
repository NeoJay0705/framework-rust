use std::sync::Mutex;

static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn record(event: &'static str) {
    EVENTS.lock().expect("event lock poisoned").push(event);
}

pub fn reset_events() {
    EVENTS.lock().expect("event lock poisoned").clear();
}

pub fn take_events() -> Vec<&'static str> {
    std::mem::take(&mut *EVENTS.lock().expect("event lock poisoned"))
}

pub struct Config(pub usize);
pub struct Database(pub usize);

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

    #[hook(start)]
    fn observe_config(_config: &crate::Config) {
        crate::record("hook.common");
    }
}
