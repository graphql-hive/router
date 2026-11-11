use bytes::Bytes as HyperBytes;
use criterion::{criterion_group, criterion_main, BatchSize, Bencher, Criterion};
use hive_router::config::coprocessor::{
    ContextSelection, CoprocessorGraphqlRequestIncludeConfig,
    CoprocessorGraphqlResponseIncludeConfig, CoprocessorHookConfig,
    CoprocessorRouterRequestIncludeConfig, CoprocessorRouterResponseIncludeConfig,
    GraphqlBodySelection,
};
use hive_router::executor::coprocessor::stage::Stage;
use hive_router::executor::coprocessor::stages::graphql::{
    GraphqlRequestInput, GraphqlRequestStage, GraphqlResponseInput, GraphqlResponseStage,
};
use hive_router::executor::coprocessor::stages::router::{
    RouterRequestInput, RouterRequestStage, RouterResponseInput, RouterResponseStage,
};
use hive_router::executor::hooks::on_graphql_params::GraphQLParams;
use hive_router::executor::request_context::SharedRequestContext;
use ntex::http::header::{HeaderName, HeaderValue};
use ntex::http::HeaderMap;
use ntex::http::{Payload, StatusCode};
use ntex::util::Bytes as NtexBytes;
use ntex::web::{self, test};
use std::cell::RefCell;
use std::hint::black_box;
use std::ops::ControlFlow;

const CONTINUE_RESPONSE_JSON: &[u8] = br#"{"version":1,"control":"continue","method":"POST","path":"/next","headers":{"x-coprocessor":["true"]},"body":"hello"}"#;
const BREAK_RESPONSE_JSON: &[u8] =
    br#"{"version":1,"control":{"break":200},"headers":{"content-type":["application/json"]},"body":"{\"ok\":true}"}"#;
const MINIMAL_CONTINUE_RESPONSE_JSON: &[u8] = br#"{"version":1,"control":"continue"}"#;
const MINIMAL_BREAK_RESPONSE_JSON: &[u8] = br#"{"version":1,"control":{"break":200}}"#;
const GRAPHQL_CONTINUE_RESPONSE_JSON: &[u8] =
    br#"{"version":1,"control":"continue","headers":{"x-coprocessor":["true"]}}"#;
const GRAPHQL_BREAK_RESPONSE_JSON: &[u8] = br#"{"version":1,"control":{"break":200},"headers":{"content-type":["application/json"]},"body":"{\"errors\":[{\"message\":\"blocked\"}]}"}"#;

fn run_stage<'a, 'b, S>(
    stage: &S,
    mut input: S::Input<'a>,
    context: &'a SharedRequestContext,
    response_bytes: &'b HyperBytes,
    id: &'a str,
) -> S::Input<'a>
where
    S: Stage,
{
    let request = <S as Stage>::build_request(
        black_box(stage),
        black_box(&input),
        black_box(id),
        black_box(context),
    )
    .expect("build_request should succeed");
    black_box(request);

    let parsed = <S as Stage>::parse_response(black_box(stage), black_box(response_bytes))
        .expect("parse_response should succeed");

    let break_decision = <S as Stage>::break_output(black_box(stage), black_box(parsed))
        .expect("break_output should succeed");

    match break_decision {
        ControlFlow::Continue(parsed) => {
            <S as Stage>::apply_mutations(stage, black_box(parsed), black_box(&mut input))
                .expect("apply_mutations should succeed");
        }
        ControlFlow::Break(response) => {
            let _ = black_box(response);
        }
    }

    // handed back so callers can reuse the request, see `RequestPool`
    black_box(input)
}

// ntex gives every `TestRequest` its own config and parks each dropped request forever in a
// thread-local pool keyed by that config. a fresh request per iteration leaks ~1KiB and criterion
// runs millions of them, so all router stage benches share one pool of reused requests. it grows
// up to the largest criterion batch and is dropped only once, when all benchmarks are done
type RequestPool = RefCell<Vec<web::WebRequest<()>>>;

// pops a pooled request, or makes the pool grow, and resets whatever the previous iteration or
// bench left on it. the stages read the body from the input bytes, never from the payload
fn take_request(
    pool: &RequestPool,
    method: &ntex::http::Method,
    headers: &HeaderMap,
) -> web::WebRequest<()> {
    let reused = pool.borrow_mut().pop();
    let mut req = reused.unwrap_or_else(|| {
        test::TestRequest::default()
            .uri("/graphql")
            .to_srv_request()
    });
    req.head_mut().method = method.clone();
    *req.headers_mut() = headers.clone();
    req.set_payload(Payload::None);
    req
}

fn make_headers(prefix: &str, header_count: usize) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for i in 0..header_count {
        let name = HeaderName::from_bytes(format!("{prefix}-{i}").as_bytes()).unwrap();
        headers.insert(name, HeaderValue::from_static("value"));
    }
    headers
}

fn bench_router_request<'a>(
    pool: &'a RequestPool,
    stage: &'a RouterRequestStage,
    method: ntex::http::Method,
    body: Option<&'static [u8]>,
    header_count: usize,
    response_json: &'static [u8],
) -> impl FnMut(&mut Bencher) + 'a {
    let headers = make_headers("x-bench", header_count);

    move |b| {
        b.iter_batched(
            || take_request(pool, &method, &headers),
            |req| {
                let input = RouterRequestInput::new(req, body.map(NtexBytes::from_static));
                let context = SharedRequestContext::default();
                let response_bytes = HyperBytes::from_static(response_json);
                let id = "id";
                let input = run_stage(
                    black_box(stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
                pool.borrow_mut().push(input.into_request());
            },
            BatchSize::SmallInput,
        )
    }
}

fn bench_router_response<'a>(
    pool: &'a RequestPool,
    stage: &'a RouterResponseStage,
    body: &'static str,
    header_count: usize,
    response_json: &'static [u8],
) -> impl FnMut(&mut Bencher) + 'a {
    let response_headers = make_headers("x-resp", header_count);
    let no_headers = HeaderMap::new();

    move |b| {
        b.iter_batched(
            || {
                let req = take_request(pool, &ntex::http::Method::GET, &no_headers);
                let mut builder = web::HttpResponse::builder(StatusCode::OK);
                for (name, value) in response_headers.iter() {
                    builder.set_header(name.clone(), value.clone());
                }
                req.into_response(builder.body(body))
            },
            |response| {
                let input = RouterResponseInput::new(response);
                let context = SharedRequestContext::default();
                let response_bytes = HyperBytes::from_static(response_json);
                let id = "id";
                let input = run_stage(
                    black_box(stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
                let (_, request) = input.into_response().into_parts();
                pool.borrow_mut()
                    .push(web::WebRequest::from_request(request));
            },
            BatchSize::SmallInput,
        )
    }
}

fn make_http_request(
    method: ntex::http::Method,
    path: &str,
    header_count: usize,
) -> web::HttpRequest {
    let mut req = test::TestRequest::default().method(method).uri(path);
    for i in 0..header_count {
        req = req.header(format!("x-bench-{i}"), "value");
    }

    req.to_http_request()
}

fn make_graphql_params() -> GraphQLParams {
    GraphQLParams {
        query: Some("query Bench{me{id}}".to_string()),
        operation_name: Some("Bench".to_string()),
        variables: Default::default(),
        extensions: Some(Default::default()),
    }
}

fn make_graphql_http_response(
    status: StatusCode,
    body: &'static [u8],
    header_count: usize,
) -> web::HttpResponse {
    let mut response = web::HttpResponse::builder(status);
    for i in 0..header_count {
        let name = HeaderName::from_bytes(format!("x-graphql-resp-{i}").as_bytes()).unwrap();
        response.set_header(name, HeaderValue::from_static("value"));
    }

    response.body(NtexBytes::from_static(body))
}

fn bench_router_request_stage(c: &mut Criterion, pool: &RequestPool) {
    let minimal_stage = RouterRequestStage::from_config(&CoprocessorHookConfig::new(
        None,
        CoprocessorRouterRequestIncludeConfig::default(),
    ))
    .expect("router.request stage should compile");
    let full_stage = RouterRequestStage::from_config(&CoprocessorHookConfig::new(
        None,
        CoprocessorRouterRequestIncludeConfig {
            body: true,
            context: ContextSelection::all(),
            headers: true,
            method: true,
            path: true,
        },
    ))
    .expect("router.request stage should compile");

    let mut group = c.benchmark_group("coprocessor/router.request");
    const BODY: &[u8] = b"{\"query\":\"{ me { id } }\"}";

    group.bench_function(
        "end_to_end/minimal_continue",
        bench_router_request(
            pool,
            &minimal_stage,
            ntex::http::Method::GET,
            None,
            8,
            MINIMAL_CONTINUE_RESPONSE_JSON,
        ),
    );

    group.bench_function(
        "end_to_end/full_continue",
        bench_router_request(
            pool,
            &full_stage,
            ntex::http::Method::POST,
            Some(BODY),
            32,
            CONTINUE_RESPONSE_JSON,
        ),
    );

    group.bench_function(
        "end_to_end/minimal_break",
        bench_router_request(
            pool,
            &minimal_stage,
            ntex::http::Method::GET,
            None,
            8,
            MINIMAL_BREAK_RESPONSE_JSON,
        ),
    );

    group.bench_function(
        "end_to_end/full_break",
        bench_router_request(
            pool,
            &full_stage,
            ntex::http::Method::POST,
            Some(BODY),
            32,
            BREAK_RESPONSE_JSON,
        ),
    );

    group.finish();
}

fn bench_router_response_stage(c: &mut Criterion, pool: &RequestPool) {
    let minimal_stage =
        RouterResponseStage::from_config(&CoprocessorHookConfig::new(None, Default::default()))
            .expect("router.response stage should compile");

    let full_stage = RouterResponseStage::from_config(&CoprocessorHookConfig::new(
        None,
        CoprocessorRouterResponseIncludeConfig {
            body: true,
            context: ContextSelection::all(),
            headers: true,
            status_code: true,
        },
    ))
    .expect("router.response stage should compile");

    let mut group = c.benchmark_group("coprocessor/router.response");
    const BODY: &str = "{\"data\":{\"hello\":\"world\"}}";

    group.bench_function(
        "end_to_end/minimal_continue",
        bench_router_response(pool, &minimal_stage, "", 8, MINIMAL_CONTINUE_RESPONSE_JSON),
    );

    group.bench_function(
        "end_to_end/full_continue",
        bench_router_response(pool, &full_stage, BODY, 32, CONTINUE_RESPONSE_JSON),
    );

    group.bench_function(
        "end_to_end/minimal_break",
        bench_router_response(pool, &minimal_stage, "", 8, MINIMAL_BREAK_RESPONSE_JSON),
    );

    group.bench_function(
        "end_to_end/full_break",
        bench_router_response(pool, &full_stage, BODY, 32, BREAK_RESPONSE_JSON),
    );

    group.finish();
}

fn bench_graphql_request_stage(c: &mut Criterion) {
    let minimal_stage =
        GraphqlRequestStage::from_config(&CoprocessorHookConfig::new(None, Default::default()))
            .expect("graphql.request stage should compile");
    let full_stage = GraphqlRequestStage::from_config(&CoprocessorHookConfig::new(
        None,
        CoprocessorGraphqlRequestIncludeConfig {
            body: GraphqlBodySelection::all(),
            context: ContextSelection::all(),
            headers: true,
            method: true,
            path: true,
            sdl: false,
        },
    ))
    .expect("graphql.request stage should compile");

    let mut group = c.benchmark_group("coprocessor/graphql.request");

    // built once per bench, see `RequestPool` for why
    let request = make_http_request(ntex::http::Method::GET, "/graphql", 8);
    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter(|| {
            let mut request_headers = request.headers().clone();
            let mut graphql_params = GraphQLParams::default();
            let context = SharedRequestContext::default();
            let input =
                GraphqlRequestInput::new(&request, &mut request_headers, &mut graphql_params, None);

            let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
            let id = "id";
            run_stage(
                black_box(&minimal_stage),
                black_box(input),
                black_box(&context),
                black_box(&response_bytes),
                black_box(id),
            );
        })
    });

    // built once per bench, see `RequestPool` for why
    let request = make_http_request(ntex::http::Method::POST, "/graphql", 32);
    group.bench_function("end_to_end/full_continue", |b| {
        b.iter(|| {
            let mut request_headers = request.headers().clone();
            let mut graphql_params = make_graphql_params();
            let context = SharedRequestContext::default();
            let input =
                GraphqlRequestInput::new(&request, &mut request_headers, &mut graphql_params, None);

            let response_bytes = HyperBytes::from_static(GRAPHQL_CONTINUE_RESPONSE_JSON);
            let id = "id";
            run_stage(
                black_box(&full_stage),
                black_box(input),
                black_box(&context),
                black_box(&response_bytes),
                black_box(id),
            );
        })
    });

    // built once per bench, see `RequestPool` for why
    let request = make_http_request(ntex::http::Method::GET, "/graphql", 8);
    group.bench_function("end_to_end/minimal_break", |b| {
        b.iter(|| {
            let mut request_headers = request.headers().clone();
            let mut graphql_params = GraphQLParams::default();
            let context = SharedRequestContext::default();
            let input =
                GraphqlRequestInput::new(&request, &mut request_headers, &mut graphql_params, None);

            let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
            let id = "id";
            run_stage(
                black_box(&minimal_stage),
                black_box(input),
                black_box(&context),
                black_box(&response_bytes),
                black_box(id),
            );
        })
    });

    // built once per bench, see `RequestPool` for why
    let request = make_http_request(ntex::http::Method::POST, "/graphql", 32);
    group.bench_function("end_to_end/full_break", |b| {
        b.iter(|| {
            let mut request_headers = request.headers().clone();
            let mut graphql_params = make_graphql_params();
            let context = SharedRequestContext::default();
            let input =
                GraphqlRequestInput::new(&request, &mut request_headers, &mut graphql_params, None);

            let response_bytes = HyperBytes::from_static(GRAPHQL_BREAK_RESPONSE_JSON);
            let id = "id";
            run_stage(
                black_box(&full_stage),
                black_box(input),
                black_box(&context),
                black_box(&response_bytes),
                black_box(id),
            );
        })
    });

    group.finish();
}

fn bench_graphql_response_stage(c: &mut Criterion) {
    let minimal_stage =
        GraphqlResponseStage::from_config(&CoprocessorHookConfig::new(None, Default::default()))
            .expect("graphql.response stage should compile");
    let full_stage = GraphqlResponseStage::from_config(&CoprocessorHookConfig::new(
        None,
        CoprocessorGraphqlResponseIncludeConfig {
            body: true,
            context: ContextSelection::all(),
            headers: true,
            sdl: false,
            status_code: true,
        },
    ))
    .expect("graphql.response stage should compile");

    let mut group = c.benchmark_group("coprocessor/graphql.response");

    let request = make_http_request(ntex::http::Method::POST, "/graphql", 32);

    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter_batched(
            || make_graphql_http_response(StatusCode::OK, b"{\"data\":{}}", 8),
            |graphql_response| {
                let context = SharedRequestContext::default();
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_continue", |b| {
        b.iter_batched(
            || make_graphql_http_response(StatusCode::OK, b"{\"data\":{\"hello\":\"world\"}}", 32),
            |graphql_response| {
                let context = SharedRequestContext::default();
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/minimal_break", |b| {
        b.iter_batched(
            || make_graphql_http_response(StatusCode::OK, b"{\"data\":{}}", 8),
            |graphql_response| {
                let context = SharedRequestContext::default();
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_break", |b| {
        b.iter_batched(
            || make_graphql_http_response(StatusCode::OK, b"{\"data\":{\"hello\":\"world\"}}", 32),
            |graphql_response| {
                let context = SharedRequestContext::default();
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(GRAPHQL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&context),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn all_benchmarks(c: &mut Criterion) {
    let pool = RequestPool::default();
    bench_router_request_stage(c, &pool);
    bench_router_response_stage(c, &pool);
    bench_graphql_request_stage(c);
    bench_graphql_response_stage(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
