use std::error::Error as _;

#[framework::application]
pub mod startup_failure_application {
    #[provide(lifecycle)]
    fn ready() -> composition_guide::StartupReady {
        composition_guide::record("construct.startup.ready");
        composition_guide::StartupReady
    }

    #[provide(lifecycle)]
    fn failure(_ready: &composition_guide::StartupReady) -> composition_guide::StartupFailure {
        composition_guide::record("construct.startup.failure");
        composition_guide::StartupFailure
    }

    #[provide(lifecycle)]
    fn never_started(
        _failure: &composition_guide::StartupFailure,
    ) -> composition_guide::StartupNeverStarted {
        composition_guide::record("construct.startup.never-started");
        composition_guide::StartupNeverStarted
    }

    #[hook(start)]
    fn hook_after_startup(_never_started: &composition_guide::StartupNeverStarted) {
        composition_guide::record("hook.startup-failure.unexpected");
    }
}

fn main() {
    let error = composition_guide::block_on(startup_failure_application::run())
        .expect_err("startup failure demo must produce RunError");

    println!("expected RunError: {error}");
    for failure in error.failures() {
        let source = failure
            .source()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<none>".to_owned());
        println!(
            "failure: phase={}, component={}, source={source}",
            failure.phase(),
            failure.component()
        );
    }
    composition_guide::print_trace();
}
