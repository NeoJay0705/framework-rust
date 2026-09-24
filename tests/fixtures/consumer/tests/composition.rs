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
