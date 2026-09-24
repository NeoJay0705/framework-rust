#[framework::application(imports(composition_guide::base_bundle))]
pub mod registration_lifecycle_application {
    #[provide(lifecycle, register(composition_guide::Router))]
    fn application_routes() -> composition_guide::AppRoutes {
        composition_guide::record("construct.route.application");
        composition_guide::AppRoutes
    }

    #[hook(start)]
    fn report_effective_route(
        router: &composition_guide::Router,
        _base_routes: &composition_guide::BaseRoutes,
        _application_routes: &composition_guide::AppRoutes,
    ) {
        let route = router
            .effective_route("home")
            .expect("base and application routes should define home");
        composition_guide::record(format!("hook.registration.home={route}"));
        composition_guide::signal_hook_complete();
    }
}

fn main() {
    let (stop_sender, stop) = composition_guide::stop_signal();
    let stop_thread = composition_guide::spawn_stop_after_hook(stop_sender);
    composition_guide::block_on(registration_lifecycle_application::run_until(stop))
        .expect("registration lifecycle demo should succeed");
    stop_thread
        .join()
        .expect("stop signal thread should finish successfully");
    composition_guide::print_trace();
}
