pub struct MissingType;
pub struct Value;
pub struct CycleA;
pub struct CycleB;

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
