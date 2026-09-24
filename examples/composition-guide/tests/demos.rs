use std::process::Command;

fn run_demo(binary: &str) -> String {
    let output = Command::new(binary)
        .output()
        .unwrap_or_else(|error| panic!("failed to run demo {binary}: {error}"));
    assert!(
        output.status.success(),
        "demo {binary} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("demo output should be UTF-8")
}

fn lines(output: &str) -> Vec<&str> {
    output.lines().collect()
}

#[test]
fn unimported_bundle_runs_only_the_local_hook() {
    let output = run_demo(env!("CARGO_BIN_EXE_unimported_bundle"));
    assert_eq!(lines(&output), ["hook.unimported.app"]);
}

#[test]
fn imported_metrics_remains_unreachable() {
    let output = run_demo(env!("CARGO_BIN_EXE_unused_import"));
    assert_eq!(lines(&output), ["hook.unused-import.app"]);
}

#[test]
fn imported_explicit_hook_runs_with_its_provider() {
    let output = run_demo(env!("CARGO_BIN_EXE_explicit_hook"));
    assert_eq!(
        lines(&output),
        ["construct.config", "hook.base", "hook.explicit.app"]
    );
}

#[test]
fn transitive_lifecycle_starts_and_shuts_down_in_dependency_order() {
    let output = run_demo(env!("CARGO_BIN_EXE_transitive_lifecycle"));
    assert_eq!(
        lines(&output),
        [
            "construct.config",
            "construct.database",
            "construct.server-backend",
            "construct.server",
            "start.lifecycle.database",
            "start.lifecycle.server",
            "hook.base",
            "hook.transitive.server",
            "shutdown.lifecycle.server",
            "shutdown.lifecycle.database",
        ]
    );
}

#[test]
fn registration_override_precedes_hooks_and_reverse_shutdown() {
    let output = run_demo(env!("CARGO_BIN_EXE_registration_lifecycle"));
    assert_eq!(
        lines(&output),
        [
            "construct.config",
            "construct.router",
            "construct.route.common",
            "construct.route.application",
            "register.route.common",
            "register.route.application",
            "start.lifecycle.base-routes",
            "start.lifecycle.app-routes",
            "hook.base",
            "hook.registration.home=application",
            "stop.signal",
            "shutdown.lifecycle.app-routes",
            "shutdown.lifecycle.base-routes",
        ]
    );
}

#[test]
fn partial_bundle_builds_only_the_selected_service() {
    let output = run_demo(env!("CARGO_BIN_EXE_partial_bundle"));
    assert_eq!(
        lines(&output),
        [
            "construct.config",
            "construct.service.primary",
            "start.lifecycle.service.primary",
            "hook.base",
            "hook.partial.service=primary",
            "shutdown.lifecycle.service.primary",
        ]
    );
}

#[test]
fn startup_failure_reports_rollback_and_skips_later_callbacks() {
    let output = run_demo(env!("CARGO_BIN_EXE_startup_failure"));
    let output_lines = lines(&output);

    assert!(output_lines[0].starts_with("expected RunError: 2 lifecycle failure(s);"));
    let failures: Vec<_> = output_lines
        .iter()
        .filter(|line| line.starts_with("failure:"))
        .copied()
        .collect();
    assert_eq!(failures.len(), 2);
    assert!(failures[0].contains("phase=start"));
    assert!(failures[0].contains("StartupFailure"));
    assert!(failures[0].contains("source=startup failed"));
    assert!(failures[1].contains("phase=shutdown"));
    assert!(failures[1].contains("StartupReady"));
    assert!(failures[1].contains("source=ready cleanup failed"));

    let expected_trace = [
        "construct.startup.ready",
        "construct.startup.failure",
        "construct.startup.never-started",
        "start.lifecycle.ready",
        "start.lifecycle.failure",
        "shutdown.lifecycle.ready",
    ];
    assert!(output_lines.ends_with(&expected_trace));
    assert!(!output.contains("shutdown.lifecycle.failure"));
    assert!(!output.contains("start.lifecycle.never-started"));
    assert!(!output.contains("shutdown.lifecycle.never-started"));
    assert!(!output.contains("hook.startup-failure.unexpected"));
}
