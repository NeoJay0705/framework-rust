#[framework::application]
pub mod unimported_application {
    #[hook(start)]
    fn app_start() {
        composition_guide::record("hook.unimported.app");
    }
}

fn main() {
    composition_guide::block_on(unimported_application::run())
        .expect("unimported bundle demo should succeed");
    composition_guide::print_trace();
}
