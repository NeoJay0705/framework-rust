pub struct OtherConfig(pub usize);

#[framework::bundle(name = other_bundle)]
pub mod other {
    #[provide]
    fn config() -> crate::OtherConfig {
        common_one::record("construct.other");
        crate::OtherConfig(31)
    }

    #[hook(start)]
    async fn observe_config(config: &crate::OtherConfig) {
        assert_eq!(config.0, 31);
        common_one::record("hook.other-async");
    }
}
