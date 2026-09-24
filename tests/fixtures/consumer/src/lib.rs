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
