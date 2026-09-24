#[framework::application(imports(composition_guide::base_bundle, composition_guide::data_bundle))]
pub mod transitive_lifecycle_application {
    #[provide]
    fn server_backend(
        database: std::sync::Arc<composition_guide::Database>,
        config: &composition_guide::Config,
    ) -> composition_guide::ServerBackend {
        composition_guide::record("construct.server-backend");
        composition_guide::ServerBackend {
            database,
            config_label: config.label,
        }
    }

    #[provide(lifecycle)]
    fn server(
        backend: &composition_guide::ServerBackend,
    ) -> std::sync::Arc<composition_guide::Server> {
        composition_guide::record("construct.server");
        std::sync::Arc::new(composition_guide::Server {
            database: std::sync::Arc::clone(&backend.database),
            config_label: backend.config_label,
        })
    }

    #[hook(start)]
    fn serve(server: std::sync::Arc<composition_guide::Server>) {
        let _ = (&server.database, server.config_label);
        composition_guide::record("hook.transitive.server");
    }
}

fn main() {
    composition_guide::block_on(transitive_lifecycle_application::run())
        .expect("transitive lifecycle demo should succeed");
    composition_guide::print_trace();
}
