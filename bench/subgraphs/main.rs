use subgraphs::start_subgraphs_server;

#[tokio::main]
async fn main() {
    let (server_handle, _shutdown_tx, _shared_state, _addr_rx) = start_subgraphs_server(None);

    server_handle
        .await
        .expect("subgraph server failed to start");
}
