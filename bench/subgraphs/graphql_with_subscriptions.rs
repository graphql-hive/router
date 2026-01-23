use std::time::Duration;
use std::{
    convert::Infallible,
    io,
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
use hive_router::pipeline::multipart_subscribe::APOLLO_MULTIPART_HTTP_CONTENT_TYPE;
use hive_router::pipeline::sse::SSE_HEADER;
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
            .is_some_and(|accept| accept.contains(SSE_HEADER));

        // for testing purposes. abruptly terminate the stream after N messages
        let break_after_count = req
            .headers()
            .get("x-break-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());

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
                let byte_stream =
                    create_sse_stream(stream, Duration::from_secs(10)).map(Ok::<_, io::Error>);
                let body = if let Some(count) = break_after_count {
                    Body::from_stream(abrupt_terminate_after(byte_stream, count))
                } else {
                    Body::from_stream(byte_stream)
                };
                Ok(HttpResponse::builder()
                    .header(http::header::CONTENT_TYPE, SSE_HEADER)
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            } else {
                let byte_stream =
                    create_multipart_subscribe_stream(stream, Duration::from_secs(30))
                        .map(Ok::<_, io::Error>);
                let body = if let Some(count) = break_after_count {
                    Body::from_stream(abrupt_terminate_after(byte_stream, count))
                } else {
                    Body::from_stream(byte_stream)
                };
                Ok(HttpResponse::builder()
                    .header(
                        http::header::CONTENT_TYPE,
                        APOLLO_MULTIPART_HTTP_CONTENT_TYPE,
                    )
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            }
        })
    }
}

fn abrupt_terminate_after<S>(
    stream: S,
    count: usize,
) -> impl Stream<Item = Result<Bytes, io::Error>>
where
    S: Stream<Item = Result<Bytes, io::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut stream = std::pin::pin!(stream);
        let mut emitted = 0;

        while let Some(item) = stream.next().await {
            yield item;
            emitted += 1;
            if emitted > count {
                // error abruptly killing the connection
                yield Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection abruptly terminated"));
                break;
            }
        }
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
    multipart_subscribe::create_apollo_multipart_http_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("Multipart stream error"))
        })
        .boxed()
}
