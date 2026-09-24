#[framework::application(imports(
    composition_guide::base_bundle,
    composition_guide::service_bundle
))]
pub mod partial_bundle_application {
    #[hook(start)]
    fn use_primary(#[inject("primary")] service: &std::sync::Arc<composition_guide::Service>) {
        composition_guide::record(format!("hook.partial.service={}", service.name));
    }
}

fn main() {
    composition_guide::block_on(partial_bundle_application::run())
        .expect("partial bundle demo should succeed");
    composition_guide::print_trace();
}
