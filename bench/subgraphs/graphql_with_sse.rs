use std::time::Duration;
use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use async_graphql::{Executor, Response as GraphQLResponse};
use async_graphql_axum::{rejection::GraphQLRejection, GraphQL as AsyncGraphQL, GraphQLRequest};
use axum::{
    body::{Body, HttpBody},
    extract::FromRequest,
    http::{Request as HttpRequest, Response as HttpResponse},
    response::IntoResponse,
    BoxError,
};
use bytes::Bytes;
use futures_util::{future::BoxFuture, stream::BoxStream, Stream, StreamExt};
use hive_router::pipeline::sse;
use tower_service::Service;

#[derive(Clone)]
pub struct GraphQL<E> {
    inner: AsyncGraphQL<E>,
    executor: E,
}

impl<E> GraphQL<E>
where
    E: Clone,
{
    pub fn new(executor: E) -> Self {
        Self {
            inner: AsyncGraphQL::new(executor.clone()),
            executor,
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

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::<HttpRequest<B>>::poll_ready(&mut self.inner, cx)
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        let is_accept_event_stream = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/event-stream"));

        if !is_accept_event_stream {
            return Service::<HttpRequest<B>>::call(&mut self.inner, req);
        }

        let executor = self.executor.clone();
        let req = req.map(Body::new);
        Box::pin(async move {
            let req = match GraphQLRequest::<GraphQLRejection>::from_request(req, &()).await {
                Ok(req) => req,
                Err(err) => return Ok(err.into_response()),
            };

            let stream = executor.execute_stream(req.0, None);

            let body = Body::from_stream(
                create_sse_stream(stream, Duration::from_secs(10)).map(Ok::<_, std::io::Error>),
            );

            Ok(HttpResponse::builder()
                .header(http::header::CONTENT_TYPE, "text/event-stream")
                .header(http::header::CACHE_CONTROL, "no-cache")
                .header(http::header::CONNECTION, "keep-alive")
                .body(body)
                .unwrap())
        })
    }
}

pub fn create_sse_stream(
    input: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> BoxStream<'static, Bytes> {
    // Convert GraphQLResponse stream to Vec<u8> stream for the router's SSE implementation
    let byte_stream =
        input.map(|resp| serde_json::to_vec(&resp).expect("Failed to serialize GraphQLResponse"));

    // Use the router's SSE stream implementation
    sse::create_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("SSE stream error"))
        })
        .boxed()
}
