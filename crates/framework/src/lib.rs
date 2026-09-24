//! Public facade for the framework.

#[doc(hidden)]
pub use framework_macros::__compose;
pub use framework_macros::{application, bundle};

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
