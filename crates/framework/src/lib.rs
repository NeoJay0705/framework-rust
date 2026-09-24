//! Public facade for the framework.

use std::{error::Error, fmt};

#[doc(hidden)]
pub use framework_macros::__compose;
pub use framework_macros::{application, bundle};

/// The error returned by a component lifecycle callback.
pub type LifecycleResult = Result<(), Box<dyn Error + Send + Sync>>;

/// Component lifecycle callbacks used by generated applications.
///
/// Both callbacks default to no-ops. The future returned by each callback is
/// not required to be `Send`.
#[allow(async_fn_in_trait)]
pub trait Lifecycle {
    /// Starts the component before application start hooks run.
    async fn start(&self) -> LifecycleResult {
        Ok(())
    }

    /// Shuts down a successfully started component.
    async fn shutdown(&self) -> LifecycleResult {
        Ok(())
    }
}

/// The lifecycle callback that produced a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecyclePhase {
    /// Component startup.
    Start,
    /// Component shutdown.
    Shutdown,
}

impl fmt::Display for LifecyclePhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => formatter.write_str("start"),
            Self::Shutdown => formatter.write_str("shutdown"),
        }
    }
}

/// A lifecycle failure associated with its phase and provider declaration.
#[derive(Debug)]
pub struct LifecycleFailure {
    phase: LifecyclePhase,
    component: String,
    error: Box<dyn Error + Send + Sync>,
}

impl LifecycleFailure {
    /// Creates a failure for generated application lifecycle code.
    #[doc(hidden)]
    pub fn new(
        phase: LifecyclePhase,
        component: impl Into<String>,
        error: Box<dyn Error + Send + Sync>,
    ) -> Self {
        Self {
            phase,
            component: component.into(),
            error,
        }
    }

    /// Returns the lifecycle phase that failed.
    pub fn phase(&self) -> LifecyclePhase {
        self.phase
    }

    /// Returns the stable provider declaration identifier.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Returns the original lifecycle error.
    pub fn error(&self) -> &(dyn Error + Send + Sync + 'static) {
        &*self.error
    }
}

impl fmt::Display for LifecycleFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "component `{}` {} failed: {}",
            self.component, self.phase, self.error
        )
    }
}

impl Error for LifecycleFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.error)
    }
}

/// Aggregates lifecycle failures in the order they occurred.
#[derive(Debug, Default)]
pub struct RunError {
    failures: Vec<LifecycleFailure>,
}

impl RunError {
    /// Creates an empty error collection for generated application code.
    #[doc(hidden)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a failure while preserving occurrence order.
    #[doc(hidden)]
    pub fn push(&mut self, failure: LifecycleFailure) {
        self.failures.push(failure);
    }

    /// Returns all lifecycle failures in occurrence order.
    pub fn failures(&self) -> &[LifecycleFailure] {
        &self.failures
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} lifecycle failure(s)", self.failures.len())?;
        for (index, failure) in self.failures.iter().enumerate() {
            write!(formatter, "; {}: {failure}", index + 1)?;
        }
        Ok(())
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.failures
            .first()
            .map(|failure| failure as &(dyn Error + 'static))
    }
}

/// Configures a framework target using a constructed component.
///
/// A registration call is generated only for a provider marked with
/// `#[provide(register(Target))]`. Both that component and its target must
/// already be reachable in the application graph; declaring a registration
/// does not add a graph root or make either value reachable.
pub trait Register<Target> {
    /// Applies this component's configuration to the target.
    fn register(&self, target: &mut Target);
}
