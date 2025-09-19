use hive_router_config::headers::{HOP_BY_HOP_HEADERS, NEVER_JOIN_HEADERS};
use http::HeaderName;

#[inline]
pub fn is_denied_header(name: &http::HeaderName) -> bool {
    HOP_BY_HOP_HEADERS.contains(&name.as_str())
}

pub fn is_never_join_header(name: &HeaderName) -> bool {
    NEVER_JOIN_HEADERS.contains(&name.as_str())
}
