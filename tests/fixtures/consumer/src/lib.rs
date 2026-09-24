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
