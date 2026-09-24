#[framework::application(imports(composition_guide::base_bundle))]
pub mod explicit_hook_application {
    #[hook(start)]
    fn app_start() {
        composition_guide::record("hook.explicit.app");
    }
}

fn main() {
    composition_guide::block_on(explicit_hook_application::run())
        .expect("explicit hook demo should succeed");
    composition_guide::print_trace();
}
