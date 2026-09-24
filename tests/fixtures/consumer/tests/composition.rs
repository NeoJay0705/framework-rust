use std::{
    error::Error as _,
    future::Future,
    pin::Pin,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
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

fn block_on_waking<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                let now = Instant::now();
                assert!(now < deadline, "timed out waiting for pending future wake");
                thread::park_timeout(deadline.saturating_duration_since(now));
            }
        }
    }
}

struct ReadyAfterPolls {
    polls: usize,
    ready_after: usize,
}

impl Future for ReadyAfterPolls {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls += 1;
        if self.polls >= self.ready_after {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

#[derive(Default)]
struct StopState {
    polls: usize,
    ready: bool,
    waker: Option<Waker>,
}

struct WakeWhenPolled {
    state: Arc<(Mutex<StopState>, Condvar)>,
}

impl Future for WakeWhenPolled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let (state_lock, condition) = &*self.state;
        let mut state = state_lock.lock().expect("stop state lock poisoned");
        state.polls += 1;
        if state.ready {
            return Poll::Ready(());
        }
        state.waker = Some(context.waker().clone());
        condition.notify_all();
        Poll::Pending
    }
}

fn lifecycle_flow_events(events: &[&'static str]) -> Vec<&'static str> {
    events
        .iter()
        .copied()
        .filter(|event| {
            event.starts_with("register.lifecycle-")
                || event.starts_with("start.lifecycle-")
                || *event == "hook.lifecycle-app"
                || event.starts_with("shutdown.lifecycle-")
        })
        .collect()
}

#[test]
fn composes_two_bundles_and_runs_reachable_providers_and_hooks_in_order() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::app::run()).expect("application should succeed");

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

    block_on(composition_consumer_fixture::zero_import_app::run())
        .expect("zero-import application should succeed");

    assert_eq!(
        common_one::take_events(),
        ["construct.zero-service", "hook.zero-service"]
    );
}

#[test]
fn composes_and_runs_an_application_with_one_import() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::single_import_app::run())
        .expect("single-import application should succeed");

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

    block_on(composition_consumer_fixture::runtime_registration_app::run())
        .expect("registration application should succeed");

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

#[test]
fn lifecycle_runs_direct_arc_and_rc_components_around_registration_and_hooks() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::lifecycle_app::run())
        .expect("one-shot lifecycle application should succeed");

    let events = common_one::take_events();
    assert_eq!(
        lifecycle_flow_events(&events),
        [
            "register.lifecycle-local",
            "start.lifecycle-common",
            "start.lifecycle-other",
            "start.lifecycle-local",
            "hook.lifecycle-app",
            "shutdown.lifecycle-local",
            "shutdown.lifecycle-other",
            "shutdown.lifecycle-common",
        ]
    );
    assert!(!events.contains(&"construct.lifecycle-unreachable"));
    assert!(!events.contains(&"start.lifecycle-unreachable"));
    assert!(!events.contains(&"shutdown.lifecycle-unreachable"));
    assert_eq!(common_one::registry_values("primary"), ["lifecycle-local"]);
}

#[test]
fn lifecycle_supports_named_and_unit_struct_providers() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::lifecycle_struct_app::run())
        .expect("struct lifecycle application should succeed");

    assert_eq!(
        common_one::take_events(),
        [
            "start.lifecycle-named-struct",
            "start.lifecycle-unit-struct",
            "hook.lifecycle-structs",
            "shutdown.lifecycle-unit-struct",
            "shutdown.lifecycle-named-struct",
        ]
    );
}

#[test]
fn run_until_skips_startup_when_stop_is_already_ready() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::lifecycle_app::run_until(
        std::future::ready(()),
    ))
    .expect("pre-stopped application should shut down cleanly");

    let events = common_one::take_events();
    assert_eq!(lifecycle_flow_events(&events), ["register.lifecycle-local"]);
    assert!(!events.contains(&"hook.lifecycle-app"));
    assert!(!events.contains(&"construct.lifecycle-unreachable"));
}

#[test]
fn run_until_stops_after_first_successful_lifecycle_start() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    block_on(composition_consumer_fixture::lifecycle_app::run_until(
        ReadyAfterPolls {
            polls: 0,
            ready_after: 2,
        },
    ))
    .expect("stop during startup should shut down cleanly");

    let events = common_one::take_events();
    assert_eq!(
        lifecycle_flow_events(&events),
        [
            "register.lifecycle-local",
            "start.lifecycle-common",
            "shutdown.lifecycle-common"
        ]
    );
    assert!(!events.contains(&"start.lifecycle-other"));
    assert!(!events.contains(&"start.lifecycle-local"));
    assert!(!events.contains(&"hook.lifecycle-app"));
}

#[test]
fn run_until_waits_for_and_wakes_on_stop_after_hooks() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    let state = Arc::new((Mutex::new(StopState::default()), Condvar::new()));
    let stop = WakeWhenPolled {
        state: Arc::clone(&state),
    };
    let wake_state = Arc::clone(&state);
    let wake_thread = thread::spawn(move || {
        let (state_lock, condition) = &*wake_state;
        let mut state = state_lock.lock().expect("stop state lock poisoned");
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.polls < 8 || state.waker.is_none() {
            let now = Instant::now();
            assert!(
                now < deadline,
                "timed out waiting for run_until's final stop poll and waker"
            );
            let remaining = deadline.saturating_duration_since(now);
            let (next_state, timeout) = condition
                .wait_timeout(state, remaining)
                .expect("stop state lock poisoned while waiting");
            state = next_state;
            assert!(
                !timeout.timed_out() || (state.polls >= 8 && state.waker.is_some()),
                "timed out waiting for run_until's final stop poll and waker"
            );
        }
        state.ready = true;
        let waker = state
            .waker
            .take()
            .expect("pending stop poll stores a waker");
        drop(state);
        waker.wake();
    });

    block_on_waking(composition_consumer_fixture::lifecycle_app::run_until(stop))
        .expect("woken stop should finish the application");
    wake_thread.join().expect("stop wake thread should finish");

    let events = common_one::take_events();
    assert_eq!(
        lifecycle_flow_events(&events),
        [
            "register.lifecycle-local",
            "start.lifecycle-common",
            "start.lifecycle-other",
            "start.lifecycle-local",
            "hook.lifecycle-app",
            "shutdown.lifecycle-local",
            "shutdown.lifecycle-other",
            "shutdown.lifecycle-common",
        ]
    );
    let common_hook = events
        .iter()
        .position(|event| *event == "hook.common")
        .expect("common bundle hook should run");
    let other_hook = events
        .iter()
        .position(|event| *event == "hook.other-async")
        .expect("other bundle hook should run");
    let lifecycle_hook = events
        .iter()
        .position(|event| *event == "hook.lifecycle-app")
        .expect("application hook should run");
    let first_shutdown = events
        .iter()
        .position(|event| *event == "shutdown.lifecycle-local")
        .expect("shutdown should follow hooks");
    assert!(common_hook < other_hook && other_hook < lifecycle_hook);
    assert!(lifecycle_hook < first_shutdown);
    assert_eq!(state.0.lock().expect("stop state lock poisoned").polls, 9);
}

#[test]
fn start_failure_rolls_back_successes_and_collects_all_shutdown_errors() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    common_one::reset_events();

    let error = block_on(composition_consumer_fixture::lifecycle_failure_app::run())
        .expect_err("startup failure should be returned");
    let failures = error.failures();
    assert_eq!(failures.len(), 3);
    assert_eq!(failures[0].phase(), framework::LifecyclePhase::Start);
    assert!(failures[0].component().contains("RollbackFailure"));
    assert!(
        failures[0]
            .component()
            .contains("application module `lifecycle_failure_app`")
    );
    assert_eq!(failures[0].error().to_string(), "start failure");
    assert_eq!(failures[0].source().unwrap().to_string(), "start failure");
    assert_eq!(failures[1].phase(), framework::LifecyclePhase::Shutdown);
    assert!(failures[1].component().contains("RollbackSecond"));
    assert!(
        failures[1]
            .component()
            .contains("application module `lifecycle_failure_app`")
    );
    assert_eq!(failures[1].error().to_string(), "second shutdown");
    assert_eq!(failures[2].phase(), framework::LifecyclePhase::Shutdown);
    assert!(failures[2].component().contains("RollbackFirst"));
    assert!(
        failures[2]
            .component()
            .contains("application module `lifecycle_failure_app`")
    );
    assert_eq!(failures[2].error().to_string(), "first shutdown");
    assert_eq!(
        common_one::take_events(),
        [
            "start.rollback-first",
            "start.rollback-second",
            "start.rollback-failure",
            "shutdown.rollback-second",
            "shutdown.rollback-first",
        ]
    );
}
