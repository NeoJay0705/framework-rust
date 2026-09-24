pub struct LocalService(pub usize);

#[framework::application]
pub mod zero_import_app {
    #[provide]
    fn local_service() -> crate::ZeroImportService {
        common_one::record("construct.zero-service");
        crate::ZeroImportService
    }

    #[hook(start)]
    fn observe_service(_service: crate::ZeroImportService) {
        common_one::record("hook.zero-service");
    }
}

pub struct ZeroImportService;

#[framework::application(imports(common_one::common_bundle))]
pub mod single_import_app {
    #[provide]
    fn local_service(config: &common_one::Config) -> crate::SingleImportService {
        common_one::record("construct.single-service");
        crate::SingleImportService(config.0)
    }

    #[hook(start)]
    fn observe_service(service: crate::SingleImportService) {
        assert_eq!(service.0, 23);
        common_one::record("hook.single-service");
    }
}

pub struct SingleImportService(pub usize);

pub struct OrderedFirst;
pub struct OrderedLater;
pub struct ArcRoutes;
pub struct RcRoutes;
pub struct QualifiedRoutes;
pub struct UnreachableAppRoutes;

impl framework::Register<common_one::Registry> for OrderedFirst {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("first");
        common_one::record("register.first");
    }
}

impl framework::Register<common_one::Registry> for OrderedLater {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("later");
        common_one::record("register.later");
    }
}

impl framework::Register<common_one::Registry> for ArcRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("arc");
        common_one::record("register.arc");
    }
}

impl framework::Register<common_one::Registry> for RcRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("rc");
        common_one::record("register.rc");
    }
}

impl framework::Register<common_one::Registry> for QualifiedRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("qualified");
        common_one::record("register.qualified");
    }
}

impl framework::Register<common_one::Registry> for UnreachableAppRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("unreachable-app");
        common_one::record("register.unreachable-app");
    }
}

#[framework::application(imports(common_one::common_bundle, common_two::other_bundle,))]
pub mod app {
    #[provide]
    fn local_service(config: &common_one::Config) -> crate::LocalService {
        common_one::record("construct.local-service");
        crate::LocalService(config.0)
    }

    #[provide]
    pub struct NamedConsumer {
        #[inject("primary")]
        database: std::sync::Arc<common_one::Database>,
    }

    #[provide]
    pub struct UnitToken;

    #[hook(start)]
    fn observe_service(service: crate::LocalService) {
        assert_eq!(service.0, 23);
        common_one::record("hook.local-service");
    }

    #[hook(start)]
    fn observe_named(named: crate::app::NamedConsumer) {
        assert_eq!(named.database.0, 23);
        common_one::record("hook.named-consumer");
    }

    #[hook(start)]
    fn observe_unit(_token: crate::app::UnitToken) {
        common_one::record("hook.unit-token");
    }

    #[hook(start)]
    fn observe_primary_database(
        #[inject("primary")] database: std::sync::Arc<common_one::Database>,
    ) {
        assert_eq!(database.0, 23);
        common_one::record("hook.primary-database");
    }
}

#[framework::application(imports(common_one::common_bundle, common_two::other_bundle))]
pub mod runtime_registration_app {
    #[provide(register(common_one::Registry, qualifier = "primary"))]
    fn ordered_first(_later: &crate::OrderedLater) -> crate::OrderedFirst {
        common_one::record("construct.first");
        crate::OrderedFirst
    }

    #[provide(register(common_one::Registry, qualifier = "primary"))]
    fn ordered_later() -> crate::OrderedLater {
        common_one::record("construct.later");
        crate::OrderedLater
    }

    #[provide(register(common_one::Registry, qualifier = "alternate"))]
    fn arc_routes() -> std::sync::Arc<crate::ArcRoutes> {
        common_one::record("construct.arc");
        std::sync::Arc::new(crate::ArcRoutes)
    }

    #[provide(register(common_one::Registry, qualifier = "alternate"))]
    fn rc_routes() -> std::rc::Rc<crate::RcRoutes> {
        common_one::record("construct.rc");
        std::rc::Rc::new(crate::RcRoutes)
    }

    #[provide(register(common_one::Registry, qualifier = "alternate"))]
    pub struct NamedRoutes {}

    #[provide(register(common_one::Registry, qualifier = "alternate"))]
    pub struct UnitRoutes;

    #[provide(register(common_one::Registry, qualifier = "alternate"))]
    fn qualified_routes() -> crate::QualifiedRoutes {
        common_one::record("construct.qualified");
        crate::QualifiedRoutes
    }

    #[provide(register(common_one::Registry, qualifier = "primary"))]
    fn unreachable_routes() -> crate::UnreachableAppRoutes {
        common_one::record("construct.unreachable-app");
        crate::UnreachableAppRoutes
    }

    #[hook(start)]
    fn observe_primary_registrations(
        #[inject("primary")] _primary: &common_one::Registry,
        _common_routes: &common_one::CommonRoutes,
        _implicit_routes: &common_one::ImplicitRoutes,
        _first: &crate::OrderedFirst,
        _later: &crate::OrderedLater,
    ) {
        common_one::record("hook.runtime-primary");
    }

    #[hook(start)]
    fn observe_alternate_registrations(
        #[inject("alternate")] _alternate: &common_one::Registry,
        _arc_routes: std::sync::Arc<crate::ArcRoutes>,
        _rc_routes: std::rc::Rc<crate::RcRoutes>,
        _named_routes: &crate::runtime_registration_app::NamedRoutes,
        _unit_routes: &crate::runtime_registration_app::UnitRoutes,
        _qualified_routes: &crate::QualifiedRoutes,
    ) {
        common_one::record("hook.runtime-alternate");
    }
}

impl framework::Register<common_one::Registry> for runtime_registration_app::NamedRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("named");
        common_one::record("register.named");
    }
}

impl framework::Register<common_one::Registry> for runtime_registration_app::UnitRoutes {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("unit");
        common_one::record("register.unit");
    }
}

pub struct LocalRcLifecycle;
pub struct UnreachableLifecycle;

impl framework::Lifecycle for LocalRcLifecycle {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.lifecycle-local");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.lifecycle-local");
        Ok(())
    }
}

impl framework::Register<common_one::Registry> for LocalRcLifecycle {
    fn register(&self, target: &mut common_one::Registry) {
        target.push("lifecycle-local");
        common_one::record("register.lifecycle-local");
    }
}

impl framework::Lifecycle for UnreachableLifecycle {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.lifecycle-unreachable");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.lifecycle-unreachable");
        Ok(())
    }
}

#[framework::application(imports(common_one::common_bundle, common_two::other_bundle))]
pub mod lifecycle_app {
    #[provide(lifecycle, register(common_one::Registry, qualifier = "primary"))]
    fn local_component() -> std::rc::Rc<crate::LocalRcLifecycle> {
        common_one::record("construct.lifecycle-local");
        std::rc::Rc::new(crate::LocalRcLifecycle)
    }

    #[provide(lifecycle)]
    fn unreachable_component() -> crate::UnreachableLifecycle {
        common_one::record("construct.lifecycle-unreachable");
        crate::UnreachableLifecycle
    }

    #[hook(start)]
    fn observe_lifecycle_components(
        #[inject("primary")] _registry: &common_one::Registry,
        _common: &common_one::CommonLifecycle,
        _other: std::sync::Arc<common_two::OtherLifecycle>,
        _local: std::rc::Rc<crate::LocalRcLifecycle>,
    ) {
        common_one::record("hook.lifecycle-app");
    }
}

#[derive(Debug)]
pub struct LifecycleTestError(pub &'static str);

impl std::fmt::Display for LifecycleTestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for LifecycleTestError {}

pub struct RollbackFirst;
pub struct RollbackSecond;
pub struct RollbackFailure;
pub struct RollbackNeverStarted;

impl framework::Lifecycle for RollbackFirst {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.rollback-first");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.rollback-first");
        Err(Box::new(LifecycleTestError("first shutdown")))
    }
}

impl framework::Lifecycle for RollbackSecond {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.rollback-second");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.rollback-second");
        Err(Box::new(LifecycleTestError("second shutdown")))
    }
}

impl framework::Lifecycle for RollbackFailure {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.rollback-failure");
        Err(Box::new(LifecycleTestError("start failure")))
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.rollback-failure");
        Ok(())
    }
}

impl framework::Lifecycle for RollbackNeverStarted {
    async fn start(&self) -> framework::LifecycleResult {
        common_one::record("start.rollback-never");
        Ok(())
    }

    async fn shutdown(&self) -> framework::LifecycleResult {
        common_one::record("shutdown.rollback-never");
        Ok(())
    }
}

#[framework::application]
pub mod lifecycle_failure_app {
    #[provide(lifecycle)]
    fn first() -> crate::RollbackFirst {
        crate::RollbackFirst
    }

    #[provide(lifecycle)]
    fn second(_first: &crate::RollbackFirst) -> crate::RollbackSecond {
        crate::RollbackSecond
    }

    #[provide(lifecycle)]
    fn fail(_second: &crate::RollbackSecond) -> crate::RollbackFailure {
        crate::RollbackFailure
    }

    #[provide(lifecycle)]
    fn never_started() -> crate::RollbackNeverStarted {
        crate::RollbackNeverStarted
    }

    #[hook(start)]
    fn observe_failure(_failure: &crate::RollbackFailure, _never: &crate::RollbackNeverStarted) {
        common_one::record("hook.rollback");
    }
}

#[framework::application]
pub mod lifecycle_struct_app {
    #[provide(lifecycle)]
    pub struct NamedLifecycleStruct {}

    impl framework::Lifecycle for NamedLifecycleStruct {
        async fn start(&self) -> framework::LifecycleResult {
            common_one::record("start.lifecycle-named-struct");
            Ok(())
        }

        async fn shutdown(&self) -> framework::LifecycleResult {
            common_one::record("shutdown.lifecycle-named-struct");
            Ok(())
        }
    }

    #[provide(lifecycle)]
    pub struct UnitLifecycleStruct;

    impl framework::Lifecycle for UnitLifecycleStruct {
        async fn start(&self) -> framework::LifecycleResult {
            common_one::record("start.lifecycle-unit-struct");
            Ok(())
        }

        async fn shutdown(&self) -> framework::LifecycleResult {
            common_one::record("shutdown.lifecycle-unit-struct");
            Ok(())
        }
    }

    #[hook(start)]
    fn observe_struct_components(
        _named: &crate::lifecycle_struct_app::NamedLifecycleStruct,
        _unit: &crate::lifecycle_struct_app::UnitLifecycleStruct,
    ) {
        common_one::record("hook.lifecycle-structs");
    }
}
