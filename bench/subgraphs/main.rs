use subgraphs::{start_subgraphs_server, HTTPStreamingSubscriptionProtocol};

#[tokio::main]
async fn main() {
    let (server_handle, _shutdown_tx, _shared_state) =
        start_subgraphs_server(None, HTTPStreamingSubscriptionProtocol::Auto, None);

    server_handle
        .await
        .expect("subgraph server failed to start");
}
