use bytes::Bytes as HyperBytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use hive_router_config::coprocessor::{
    CoprocessorGraphqlRequestIncludeConfig, CoprocessorGraphqlResponseIncludeConfig,
    CoprocessorHookConfig, CoprocessorRouterRequestIncludeConfig,
    CoprocessorRouterResponseIncludeConfig, GraphqlBodySelection,
};
use hive_router_plan_executor::coprocessor::stage::Stage;
use hive_router_plan_executor::coprocessor::stages::graphql::{
    GraphqlRequestInput, GraphqlRequestStage, GraphqlResponseInput, GraphqlResponseStage,
};
use hive_router_plan_executor::coprocessor::stages::router::{
    RouterRequestInput, RouterRequestStage, RouterResponseInput, RouterResponseStage,
};
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use ntex::http::header::{HeaderName, HeaderValue};
use ntex::http::StatusCode;
use ntex::util::Bytes as NtexBytes;
use ntex::web::{self, test, DefaultError};
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
    response_bytes: &'b HyperBytes,
    id: &'a str,
) where
    S: Stage,
{
    let request = <S as Stage>::build_request(black_box(stage), black_box(&input), black_box(id))
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
            let _ = black_box(input);
        }
        ControlFlow::Break(response) => {
            let _ = black_box(response);
        }
    }
}

fn make_web_request(
    method: ntex::http::Method,
    path: &str,
    body: &'static [u8],
    header_count: usize,
) -> web::WebRequest<DefaultError> {
    let mut req = test::TestRequest::default()
        .method(method)
        .uri(path)
        .set_payload(body);

    for i in 0..header_count {
        req = req.header(format!("x-bench-{i}"), "value");
    }

    req.to_srv_request()
}

fn make_web_response(
    status_code: ntex::http::StatusCode,
    body: &'static str,
    header_count: usize,
) -> web::WebResponse {
    let req = test::TestRequest::default().to_srv_request();
    let mut builder = web::HttpResponse::build(status_code);
    for i in 0..header_count {
        let name = HeaderName::from_bytes(format!("x-resp-{i}").as_bytes()).unwrap();
        builder.set_header(name, HeaderValue::from_static("value"));
    }

    req.into_response(builder.body(body))
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
    let mut response = web::HttpResponse::build(status);
    for i in 0..header_count {
        let name = HeaderName::from_bytes(format!("x-graphql-resp-{i}").as_bytes()).unwrap();
        response.set_header(name, HeaderValue::from_static("value"));
    }

    response.body(NtexBytes::from_static(body))
}

fn bench_router_request_stage(c: &mut Criterion) {
    let minimal_stage = RouterRequestStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: CoprocessorRouterRequestIncludeConfig::default(),
    })
    .expect("router.request stage should compile");
    let full_stage = RouterRequestStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: CoprocessorRouterRequestIncludeConfig {
            body: true,
            context: true,
            headers: true,
            method: true,
            path: true,
        },
    })
    .expect("router.request stage should compile");

    let mut group = c.benchmark_group("coprocessor/router.request");

    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter_batched(
            || make_web_request(ntex::http::Method::GET, "/graphql", b"", 8),
            |req| {
                let input = RouterRequestInput::new(req, None);
                let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_continue", |b| {
        const BODY: &[u8] = b"{\"query\":\"{ me { id } }\"}";
        b.iter_batched(
            || make_web_request(ntex::http::Method::POST, "/graphql", BODY, 32),
            |req| {
                let input = RouterRequestInput::new(req, Some(NtexBytes::from_static(BODY)));
                let response_bytes = HyperBytes::from_static(CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/minimal_break", |b| {
        b.iter_batched(
            || make_web_request(ntex::http::Method::GET, "/graphql", b"", 8),
            |req| {
                let input = RouterRequestInput::new(req, None);
                let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_break", |b| {
        const BODY: &[u8] = b"{\"query\":\"{ me { id } }\"}";
        b.iter_batched(
            || make_web_request(ntex::http::Method::POST, "/graphql", BODY, 32),
            |req| {
                let input = RouterRequestInput::new(req, Some(NtexBytes::from_static(BODY)));
                let response_bytes = HyperBytes::from_static(BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_router_response_stage(c: &mut Criterion) {
    let minimal_stage = RouterResponseStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: Default::default(),
    })
    .expect("router.response stage should compile");

    let full_stage = RouterResponseStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: CoprocessorRouterResponseIncludeConfig {
            body: true,
            context: true,
            headers: true,
            status_code: true,
        },
    })
    .expect("router.response stage should compile");

    let mut group = c.benchmark_group("coprocessor/router.response");

    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter_batched(
            || make_web_response(ntex::http::StatusCode::OK, "", 8),
            |response| {
                let input = RouterResponseInput::new(response);
                let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_continue", |b| {
        b.iter_batched(
            || {
                make_web_response(
                    ntex::http::StatusCode::OK,
                    "{\"data\":{\"hello\":\"world\"}}",
                    32,
                )
            },
            |response| {
                let input = RouterResponseInput::new(response);
                let response_bytes = HyperBytes::from_static(CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/minimal_break", |b| {
        b.iter_batched(
            || make_web_response(ntex::http::StatusCode::OK, "", 8),
            |response| {
                let input = RouterResponseInput::new(response);
                let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_break", |b| {
        b.iter_batched(
            || {
                make_web_response(
                    ntex::http::StatusCode::OK,
                    "{\"data\":{\"hello\":\"world\"}}",
                    32,
                )
            },
            |response| {
                let input = RouterResponseInput::new(response);
                let response_bytes = HyperBytes::from_static(BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_graphql_request_stage(c: &mut Criterion) {
    let minimal_stage = GraphqlRequestStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: Default::default(),
    })
    .expect("graphql.request stage should compile");
    let full_stage = GraphqlRequestStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: CoprocessorGraphqlRequestIncludeConfig {
            body: GraphqlBodySelection::all(),
            context: true,
            headers: true,
            method: true,
            path: true,
            sdl: false,
        },
    })
    .expect("graphql.request stage should compile");

    let mut group = c.benchmark_group("coprocessor/graphql.request");

    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter_batched(
            || make_http_request(ntex::http::Method::GET, "/graphql", 8),
            |request| {
                let mut request_headers = request.headers().clone();
                let mut graphql_params = GraphQLParams::default();
                let input = GraphqlRequestInput::new(
                    &request,
                    &mut request_headers,
                    &mut graphql_params,
                    None,
                );

                let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_continue", |b| {
        b.iter_batched(
            || make_http_request(ntex::http::Method::POST, "/graphql", 32),
            |request| {
                let mut request_headers = request.headers().clone();
                let mut graphql_params = make_graphql_params();
                let input = GraphqlRequestInput::new(
                    &request,
                    &mut request_headers,
                    &mut graphql_params,
                    None,
                );

                let response_bytes = HyperBytes::from_static(GRAPHQL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/minimal_break", |b| {
        b.iter_batched(
            || make_http_request(ntex::http::Method::GET, "/graphql", 8),
            |request| {
                let mut request_headers = request.headers().clone();
                let mut graphql_params = GraphQLParams::default();
                let input = GraphqlRequestInput::new(
                    &request,
                    &mut request_headers,
                    &mut graphql_params,
                    None,
                );

                let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("end_to_end/full_break", |b| {
        b.iter_batched(
            || make_http_request(ntex::http::Method::POST, "/graphql", 32),
            |request| {
                let mut request_headers = request.headers().clone();
                let mut graphql_params = make_graphql_params();
                let input = GraphqlRequestInput::new(
                    &request,
                    &mut request_headers,
                    &mut graphql_params,
                    None,
                );

                let response_bytes = HyperBytes::from_static(GRAPHQL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
                    black_box(&response_bytes),
                    black_box(id),
                );
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_graphql_response_stage(c: &mut Criterion) {
    let minimal_stage = GraphqlResponseStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: Default::default(),
    })
    .expect("graphql.response stage should compile");
    let full_stage = GraphqlResponseStage::from_config(&CoprocessorHookConfig {
        condition: None,
        include: CoprocessorGraphqlResponseIncludeConfig {
            body: true,
            context: true,
            headers: true,
            sdl: false,
            status_code: true,
        },
    })
    .expect("graphql.response stage should compile");

    let mut group = c.benchmark_group("coprocessor/graphql.response");

    let request = make_http_request(ntex::http::Method::POST, "/graphql", 32);

    group.bench_function("end_to_end/minimal_continue", |b| {
        b.iter_batched(
            || make_graphql_http_response(StatusCode::OK, b"{\"data\":{}}", 8),
            |graphql_response| {
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(MINIMAL_CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
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
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(CONTINUE_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
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
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(MINIMAL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&minimal_stage),
                    black_box(input),
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
                let input = GraphqlResponseInput::new(graphql_response, &request, None);

                let response_bytes = HyperBytes::from_static(GRAPHQL_BREAK_RESPONSE_JSON);
                let id = "id";
                run_stage(
                    black_box(&full_stage),
                    black_box(input),
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
    bench_router_request_stage(c);
    bench_router_response_stage(c);
    bench_graphql_request_stage(c);
    bench_graphql_response_stage(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
