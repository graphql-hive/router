use http::{HeaderName, HeaderValue, Request};
use tower_http::request_id::{MakeRequestId, RequestId};
use ulid::Ulid;

pub static REQUEST_ID_HEADER_NAME: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Debug, Clone)]
pub struct RequestIdGenerator;

impl MakeRequestId for RequestIdGenerator {
    fn make_request_id<B>(&mut self, request: &Request<B>) -> Option<RequestId> {
        let request_id = request
            .headers()
            .get(&REQUEST_ID_HEADER_NAME)
            .cloned()
            .unwrap_or_else(|| {
                let as_str = Ulid::new().to_string();
                HeaderValue::from_str(&as_str).unwrap()
            });

        Some(RequestId::new(request_id))
    }
}
