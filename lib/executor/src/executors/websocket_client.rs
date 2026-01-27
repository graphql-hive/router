use ntex::{
    io::Sealed,
    ws::{
        error::{WsClientBuilderError, WsClientError},
        WsClient, WsConnection,
    },
};
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum WsConnectError {
    #[error("WebSocket client error: {0}")]
    Client(#[from] WsClientError),
    #[error("WebSocket client builder error: {0}")]
    Builder(#[from] WsClientBuilderError),
}

pub async fn connect(url: &str) -> Result<WsConnection<Sealed>, WsConnectError> {
    if url.starts_with("wss://") {
        use tls_openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::PEER);
        let _ = builder
            .set_alpn_protos(b"\x08http/1.1")
            .map_err(|e| error!("Cannot set alpn protocol: {e:?}"));

        let ws_client = WsClient::build(url)
            .timeout(ntex::time::Seconds(60))
            .openssl(builder.build())
            .take()
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    } else {
        let ws_client = WsClient::build(url)
            .timeout(ntex::time::Seconds(60))
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    }
}

pub struct GraphQLTransportWSClient {
    connection: WsConnection<Sealed>,
}

impl GraphQLTransportWSClient {
    pub fn new(connection: WsConnection<Sealed>) -> Self {
        Self { connection }
    }

    // TODO: implement sending and receiving messages over the websocket client
}
