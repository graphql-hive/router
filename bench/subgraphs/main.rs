use subgraphs::{start_subgraphs_server, SubscriptionProtocol};

#[tokio::main]
async fn main() {
    let (server_handle, _shutdown_tx, _shared_state) =
        start_subgraphs_server(None, SubscriptionProtocol::Auto, None);

    server_handle
        .await
        .expect("subgraph server failed to start");
}
