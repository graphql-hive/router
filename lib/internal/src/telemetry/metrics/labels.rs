pub const RESULT: &str = "result";
pub const STATUS: &str = "status";
pub const ERROR_TYPE: &str = "error.type";
pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
pub const HTTP_ROUTE: &str = "http.route";
pub const URL_SCHEME: &str = "url.scheme";
pub const NETWORK_PROTOCOL_NAME: &str = "network.protocol.name";
pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
pub const SERVER_ADDRESS: &str = "server.address";
pub const SERVER_PORT: &str = "server.port";

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum SupergraphPollResult {
    #[strum(serialize = "updated")]
    Updated,
    #[strum(serialize = "not_modified")]
    NotModified,
    #[strum(serialize = "error")]
    Error,
}

impl SupergraphPollResult {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum SupergraphProcessStatus {
    #[strum(serialize = "ok")]
    Ok,
    #[strum(serialize = "error")]
    Error,
}

impl SupergraphProcessStatus {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum CacheResult {
    #[strum(serialize = "hit")]
    Hit,
    #[strum(serialize = "miss")]
    Miss,
}

impl CacheResult {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}
