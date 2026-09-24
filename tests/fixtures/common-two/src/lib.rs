pub struct OtherConfig(pub usize);
pub struct OtherLifecycle;

impl framework::Lifecycle for OtherLifecycle {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.lifecycle-other");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.lifecycle-other");
        Ok(())
    }
}

#[framework::bundle(name = other_bundle)]
pub mod other {
    #[provide]
    fn config() -> crate::OtherConfig {
        common_one::record("construct.other");
        crate::OtherConfig(31)
    }

    #[provide(lifecycle)]
    fn lifecycle_component() -> std::sync::Arc<crate::OtherLifecycle> {
        common_one::record("construct.lifecycle-other");
        std::sync::Arc::new(crate::OtherLifecycle)
    }

    #[hook(start)]
    async fn observe_config(config: &crate::OtherConfig) {
        assert_eq!(config.0, 31);
        common_one::record("hook.other-async");
    }
}
