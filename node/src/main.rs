use std::sync::Arc;

use actix_web::{
    middleware::{self, Logger},
    web::Data,
    App as ActixApp, HttpServer,
};
use app::App;
use network::{api, management, raft, raft_network_impl::Network};
use openraft::{BasicNode, Config};
use store::{Request, Response, Store};
use tracing::info;
use tracing_subscriber::EnvFilter;

pub mod app;
pub mod network;
pub mod store;

pub type Raft = openraft::Raft<TypeConfig, Network, Arc<Store>>;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig: D = Request, R = Response, NodeId = NodeId, Node = BasicNode
);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Setup the logger
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Get node id from env variable
    let node_id = std::env::var("NODE_ID")
        .expect("NODE_ID env variable is not set")
        .parse::<NodeId>()
        .expect("NODE_ID env variable is not a valid integer");

    // Get http address from env variable
    let http_addr = std::env::var("HTTP_ADDR").expect("HTTP_ADDR env variable is not set");

    info!("Starting node {} on {}", node_id, http_addr);

    // Configure raft node
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        cluster_name: "pulsedb".to_string(),
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = Arc::new(Store::default());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Network::default();

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, store.clone())
        .await
        .unwrap();

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let app = Data::new(App {
        id: node_id,
        addr: http_addr.clone(),
        raft,
        store,
        config,
    });

    // Start the actix-web server.
    let server = HttpServer::new(move || {
        ActixApp::new()
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(middleware::Compress::default())
            .app_data(app.clone())
            // raft internal RPC
            .service(raft::append)
            .service(raft::snapshot)
            .service(raft::vote)
            // admin API
            .service(management::init)
            .service(management::add_learner)
            .service(management::change_membership)
            .service(management::metrics)
            // application API
            .service(api::write)
            .service(api::read)
            .service(api::consistent_read)
    })
    .bind(http_addr)?;

    server.run().await
}
