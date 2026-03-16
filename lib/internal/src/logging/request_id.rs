use std::{fmt::Display, sync::LazyLock};

use ntex::web::HttpRequest;
use opentelemetry::trace::TraceContextExt;
use sonyflake::Sonyflake;

use crate::telemetry::otel;

static REQUEST_ID_HEADER: &str = "x-request-id";

static SONYFLAKE: LazyLock<Sonyflake> =
    LazyLock::new(|| Sonyflake::new().expect("Failed to create Sonyflake"));

pub fn obtain_req_correlation_id<'req>(
    request: &'req HttpRequest,
    otel_ctx: &otel::opentelemetry::Context,
) -> RequestIdentifier<'req> {
    // Our preference is to use identifiers that are based on OTEL/W3C trace context, if those are available.
    // In case it doesn't, we'll fallback to `x-request-id` header or generate a new one.
    {
        let span_ref = otel_ctx.span();
        let context_ref = span_ref.span_context();

        if context_ref.is_valid() {
            return RequestIdentifier::FromOTELContext(context_ref.trace_id());
        }
    }

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
    FromOTELContext(opentelemetry::trace::TraceId),
}

impl Display for RequestIdentifier<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestIdentifier::FromRequest(id) => write!(f, "{}", id),
            RequestIdentifier::Generated(id) => write!(f, "{}", id),
            RequestIdentifier::FromOTELContext(id) => write!(f, "{}", id),
        }
    }
}
