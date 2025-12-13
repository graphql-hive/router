use std::time::Duration;
use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use async_graphql::{http::is_accept_multipart_mixed, Executor, Response as GraphQLResponse};
use async_graphql_axum::GraphQLBatchRequest;
use async_graphql_axum::{rejection::GraphQLRejection, GraphQLRequest};
use axum::{
    body::{Body, HttpBody},
    extract::FromRequest,
    http::{Request as HttpRequest, Response as HttpResponse},
    response::IntoResponse,
    BoxError,
};
use bytes::Bytes;
use futures_util::{future::BoxFuture, stream::BoxStream, Stream, StreamExt};
use hive_router::pipeline::{multipart_subscribe, sse};
use tower_service::Service;

use crate::SubscriptionProtocol;

#[derive(Clone)]
pub struct GraphQL<E> {
    executor: E,
    subscriptions_protocol: SubscriptionProtocol,
}

impl<E> GraphQL<E>
where
    E: Clone,
{
    pub fn new(executor: E, subscriptions_protocol: SubscriptionProtocol) -> Self {
        Self {
            executor,
            subscriptions_protocol,
        }
    }
}

impl<B, E> Service<HttpRequest<B>> for GraphQL<E>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Data: Into<Bytes>,
    B::Error: Into<BoxError>,
    E: Executor,
{
    type Response = HttpResponse<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        let does_accept_multipart_mixed = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .map(is_accept_multipart_mixed)
            .unwrap_or_default();

        let does_accept_event_stream = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/event-stream"));

        let sub_prot = self.subscriptions_protocol.clone();

        let executor = self.executor.clone();
        let req = req.map(Body::new);

        Box::pin(async move {
            if !does_accept_multipart_mixed && !does_accept_event_stream {
                let req =
                    match GraphQLBatchRequest::<GraphQLRejection>::from_request(req, &()).await {
                        Ok(req) => req,
                        Err(err) => return Ok(err.into_response()),
                    };
                return Ok(async_graphql_axum::GraphQLResponse(
                    executor.execute_batch(req.0).await,
                )
                .into_response());
            }

            let req = match GraphQLRequest::<GraphQLRejection>::from_request(req, &()).await {
                Ok(req) => req,
                Err(err) => return Ok(err.into_response()),
            };
            let stream = executor.execute_stream(req.0, None);

            let use_sse = match sub_prot {
                SubscriptionProtocol::Auto => does_accept_event_stream,
                SubscriptionProtocol::SseOnly => true,
                SubscriptionProtocol::MultipartOnly => false,
            };

            if use_sse {
                let body = Body::from_stream(
                    create_sse_stream(stream, Duration::from_secs(10)).map(Ok::<_, std::io::Error>),
                );
                Ok(HttpResponse::builder()
                    .header(http::header::CONTENT_TYPE, "text/event-stream")
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            } else {
                let body = Body::from_stream(
                    create_multipart_subscribe_stream(stream, Duration::from_secs(30))
                        .map(Ok::<_, std::io::Error>),
                );
                Ok(HttpResponse::builder()
                    .header(
                        http::header::CONTENT_TYPE,
                        "multipart/mixed; boundary=graphql",
                    )
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            }
        })
    }
}

pub fn create_sse_stream(
    input: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> BoxStream<'static, Bytes> {
    // GraphQLResponse stream to Vec<u8> stream
    let byte_stream =
        input.map(|resp| serde_json::to_vec(&resp).expect("Failed to serialize GraphQLResponse"));
    sse::create_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("SSE stream error"))
        })
        .boxed()
}

pub fn create_multipart_subscribe_stream(
    input: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> BoxStream<'static, Bytes> {
    // GraphQLResponse stream to Vec<u8> stream
    let byte_stream =
        input.map(|resp| serde_json::to_vec(&resp).expect("Failed to serialize GraphQLResponse"));
    multipart_subscribe::create_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("Multipart stream error"))
        })
        .boxed()
}
