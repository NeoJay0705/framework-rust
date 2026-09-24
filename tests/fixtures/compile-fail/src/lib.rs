pub struct MissingType;
pub struct Value;
pub struct CycleA;
pub struct CycleB;
pub struct RegistrationTarget;
pub struct RegistrationProvider;
pub struct NoRegistrationImpl;
pub struct MissingRegistrationTarget;
pub struct SelfTarget;

impl framework::Register<RegistrationTarget> for RegistrationProvider {
    fn register(&self, _: &mut RegistrationTarget) {}
}

impl framework::Register<MissingRegistrationTarget> for RegistrationProvider {
    fn register(&self, _: &mut MissingRegistrationTarget) {}
}

impl framework::Register<SelfTarget> for SelfTarget {
    fn register(&self, _: &mut SelfTarget) {}
}

#[cfg(feature = "case-missing")]
#[framework::application]
mod missing {
    #[hook(start)]
    fn start(_value: crate::MissingType) {}
}

#[cfg(feature = "case-ambiguous")]
#[framework::application]
mod ambiguous {
    #[provide(qualifier = "primary")]
    fn primary() -> crate::Value {
        crate::Value
    }

    #[provide(qualifier = "secondary")]
    fn secondary() -> crate::Value {
        crate::Value
    }

    #[hook(start)]
    fn start(_value: crate::Value) {}
}

#[cfg(feature = "case-cycle")]
#[framework::application]
mod cycle {
    #[provide]
    fn first(_second: crate::CycleB) -> crate::CycleA {
        crate::CycleA
    }

    #[provide]
    fn second(_first: crate::CycleA) -> crate::CycleB {
        crate::CycleB
    }

    #[hook(start)]
    fn start(_first: crate::CycleA) {}
}

#[cfg(feature = "case-invalid-qualifier")]
#[framework::application]
mod invalid_qualifier {
    #[provide(qualifier = "primary")]
    fn primary() -> crate::Value {
        crate::Value
    }

    #[hook(start)]
    fn start(#[inject("missing")] _value: crate::Value) {}
}

#[cfg(feature = "case-unsupported-syntax")]
#[framework::application]
mod unsupported_syntax {
    #[provide]
    fn generic<T>() -> crate::Value {
        let _ = std::marker::PhantomData::<T>;
        crate::Value
    }

    #[hook(start)]
    fn start(_value: crate::Value) {}
}

#[cfg(feature = "case-duplicate-provider")]
#[framework::application]
mod duplicate_provider {
    #[provide]
    fn first() -> crate::Value {
        crate::Value
    }

    #[provide]
    fn second() -> crate::Value {
        crate::Value
    }

    #[hook(start)]
    fn start(_value: crate::Value) {}
}

#[cfg(feature = "case-missing-register-impl")]
#[framework::application]
mod missing_registration_impl {
    #[provide]
    fn target() -> crate::RegistrationTarget {
        crate::RegistrationTarget
    }

    #[provide(register(crate::RegistrationTarget))]
    fn registrar() -> crate::NoRegistrationImpl {
        crate::NoRegistrationImpl
    }

    #[hook(start)]
    fn start(_: &crate::RegistrationTarget, _: &crate::NoRegistrationImpl) {}
}

#[cfg(feature = "case-missing-register-impl-unreachable")]
#[framework::application]
mod missing_unreachable_registration_impl {
    #[provide(register(crate::RegistrationTarget))]
    fn registrar() -> crate::NoRegistrationImpl {
        crate::NoRegistrationImpl
    }
}

#[cfg(feature = "case-registration-target-missing")]
#[framework::application]
mod registration_target_missing {
    #[provide(register(crate::MissingRegistrationTarget))]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }

    #[hook(start)]
    fn start(_: &crate::RegistrationProvider) {}
}

#[cfg(feature = "case-registration-target-ambiguous")]
#[framework::application]
mod registration_target_ambiguous {
    #[provide(qualifier = "primary")]
    fn primary() -> crate::RegistrationTarget {
        crate::RegistrationTarget
    }

    #[provide(qualifier = "secondary")]
    fn secondary() -> crate::RegistrationTarget {
        crate::RegistrationTarget
    }

    #[provide(register(crate::RegistrationTarget))]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }

    #[hook(start)]
    fn start(_: &crate::RegistrationProvider) {}
}

#[cfg(feature = "case-registration-target-invalid-qualifier")]
#[framework::application]
mod registration_target_invalid_qualifier {
    #[provide(qualifier = "available")]
    fn target() -> crate::RegistrationTarget {
        crate::RegistrationTarget
    }

    #[provide(register(crate::RegistrationTarget, qualifier = "missing"))]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }

    #[hook(start)]
    fn start(_: &crate::RegistrationProvider) {}
}

#[cfg(feature = "case-registration-target-unreachable")]
#[framework::application]
mod registration_target_unreachable {
    #[provide]
    fn target() -> crate::RegistrationTarget {
        crate::RegistrationTarget
    }

    #[provide(register(crate::RegistrationTarget))]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }

    #[hook(start)]
    fn start(_: &crate::RegistrationProvider) {}
}

#[cfg(feature = "case-registration-self-target")]
#[framework::application]
mod registration_self_target {
    #[provide(register(crate::SelfTarget))]
    fn target() -> crate::SelfTarget {
        crate::SelfTarget
    }

    #[hook(start)]
    fn start(_: &crate::SelfTarget) {}
}

#[cfg(feature = "case-duplicate-registration")]
#[framework::application]
mod duplicate_registration {
    #[provide(
        register(crate::RegistrationTarget),
        register(crate::RegistrationTarget)
    )]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }
}

#[cfg(feature = "case-registration-bad-syntax")]
#[framework::application]
mod registration_bad_syntax {
    #[provide(register(crate::RegistrationTarget, bad = "unknown"))]
    fn registrar() -> crate::RegistrationProvider {
        crate::RegistrationProvider
    }
}
