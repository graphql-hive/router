use std::{fmt::Display, sync::LazyLock};

use ntex::web::HttpRequest;
use sonyflake::Sonyflake;

static REQUEST_ID_HEADER: &str = "x-request-id";

static SONYFLAKE: LazyLock<Sonyflake> =
    LazyLock::new(|| Sonyflake::new().expect("Failed to create Sonyflake"));

pub fn obtain_req_correlation_id<'req>(request: &'req HttpRequest) -> RequestIdentifier<'req> {
    if let Some(req_id_header) = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        return RequestIdentifier::FromRequest(req_id_header);
    }

    if let Ok(id) = SONYFLAKE.next_id() {
        return RequestIdentifier::Generated(id);
    }

    RequestIdentifier::Generated(0)
}

pub enum RequestIdentifier<'a> {
    FromRequest(&'a str),
    Generated(u64),
}

impl Display for RequestIdentifier<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestIdentifier::FromRequest(id) => write!(f, "{}", id),
            RequestIdentifier::Generated(id) => write!(f, "{}", id),
        }
    }
}
