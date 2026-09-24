#[framework::application(imports(composition_guide::metrics_bundle))]
pub mod unused_import_application {
    #[hook(start)]
    fn app_start() {
        composition_guide::record("hook.unused-import.app");
    }
}

fn main() {
    composition_guide::block_on(unused_import_application::run())
        .expect("unused import demo should succeed");
    composition_guide::print_trace();
}
