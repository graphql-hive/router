use hive_router_config::headers::{HOP_BY_HOP_HEADERS, NEVER_JOIN_HEADERS};
use http::HeaderName;

const ROUTER_OWNED_RESPONSE_HEADERS: &[&str] = &["content-type"];

#[inline]
pub fn is_denied_header(name: &http::HeaderName) -> bool {
    HOP_BY_HOP_HEADERS.contains(&name.as_str())
}

#[inline]
pub fn is_denied_response_header(name: &http::HeaderName) -> bool {
    is_denied_header(name) || ROUTER_OWNED_RESPONSE_HEADERS.contains(&name.as_str())
}

pub fn is_never_join_header(name: &HeaderName) -> bool {
    NEVER_JOIN_HEADERS.contains(&name.as_str())
}
