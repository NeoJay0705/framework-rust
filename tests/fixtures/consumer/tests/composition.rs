use std::{
    future::Future,
    sync::Mutex,
    task::{Context, Poll, Waker},
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixture hooks should not suspend"),
    }
}

#[test]
fn composes_two_bundles_and_runs_reachable_providers_and_hooks_in_order() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::app::run());

    assert_eq!(
        common_one::take_events(),
        [
            "construct.config",
            "construct.primary",
            "construct.other",
            "construct.local-service",
            "hook.common",
            "hook.other-async",
            "hook.local-service",
            "hook.named-consumer",
            "hook.unit-token",
            "hook.primary-database",
        ]
    );
}

#[test]
fn composes_and_runs_an_application_with_no_imports() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::zero_import_app::run());

    assert_eq!(
        common_one::take_events(),
        ["construct.zero-service", "hook.zero-service"]
    );
}

#[test]
fn composes_and_runs_an_application_with_one_import() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::single_import_app::run());

    assert_eq!(
        common_one::take_events(),
        [
            "construct.config",
            "construct.single-service",
            "hook.common",
            "hook.single-service",
        ]
    );
}

#[test]
fn registers_reachable_components_before_hooks_with_stable_dependency_order() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::runtime_registration_app::run());

    let events = common_one::take_events();
    assert_eq!(
        events,
        [
            "construct.config",
            "construct.registry-primary",
            "construct.registry-alternate",
            "construct.common-routes",
            "construct.implicit-routes",
            "construct.other",
            "construct.later",
            "construct.first",
            "construct.arc",
            "construct.rc",
            "construct.qualified",
            "register.common",
            "register.later",
            "register.first",
            "register.arc",
            "register.rc",
            "register.named",
            "register.unit",
            "register.qualified",
            "hook.common",
            "hook.other-async",
            "hook.runtime-primary",
            "hook.runtime-alternate",
        ]
    );
    assert!(!events.contains(&"register.implicit-discovered"));
    assert_eq!(
        common_one::registry_values("primary"),
        ["common", "later", "first"]
    );
    assert_eq!(common_one::registry_effective("primary"), Some("first"));
    assert_eq!(
        common_one::registry_values("alternate"),
        ["arc", "rc", "named", "unit", "qualified"]
    );
}
