use std::collections::HashMap;
use std::sync::Arc;
use std::{env, vec};

use actix_web::http::Method;
use actix_web::web::Html;
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use query_plan_executor::{execute_query_plan, ExecutionResult};
use query_plan_executor::{ExecutionRequest, GraphQLError};
use query_planner::ast::normalization::normalize_operation;
use query_planner::planner::Planner;
use query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use query_planner::utils::parsing::parse_schema;
use query_planner::utils::parsing::safe_parse_operation;
use serde_json::json;
use serde_json::Value::{self};
use tracing::{debug, error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[actix_web::main]
async fn main() {
    let tree_layer = tracing_tree::HierarchicalLayer::new(2)
        .with_bracketed_fields(true)
        .with_deferred_spans(false)
        .with_wraparound(25)
        .with_indent_lines(true)
        .with_timer(tracing_tree::time::Uptime::default())
        .with_thread_names(false)
        .with_thread_ids(false)
        .with_targets(false);

    tracing_subscriber::registry()
        .with(tree_layer)
        .with(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();

    let supergraph_path = &args[1];
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let planner =
        Planner::new_from_supergraph_state(&supergraph_state).expect("failed to create planner");
    let serve_data = ServeData {
        supergraph_source: supergraph_path.to_string(),
        planner,
        validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
        subgraph_endpoint_map: supergraph_state.subgraph_endpoint_map,
        http_client: reqwest::Client::new(),
        plan_cache: moka::future::Cache::new(1000),
        validate_cache: moka::future::Cache::new(1000),
    };
    let serve_data_arc = Arc::new(serve_data);
    info!("Starting server on http://localhost:4000");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(serve_data_arc.clone()))
            .service(graphql_get)
            .service(graphql_post)
            .default_service(web::route().to(landing))
    })
    .bind(("0.0.0.0", 4000))
    .expect("Failed to bind server")
    .run()
    .await
    .expect("Failed to run server")
}

struct ServeData {
    supergraph_source: String,
    planner: Planner,
    validation_plan: graphql_tools::validation::validate::ValidationPlan,
    subgraph_endpoint_map: HashMap<String, String>,
    http_client: reqwest::Client,
    plan_cache: moka::future::Cache<u64, Arc<query_planner::planner::plan_nodes::QueryPlan>>,
    validate_cache:
        moka::future::Cache<u64, Arc<Vec<graphql_tools::validation::utils::ValidationError>>>,
}

static LANDING_PAGE_HTML: &str = include_str!("../static/landing-page.html");
static __PRODUCT_LOGO__: &str = include_str!("../static/product_logo.svg");

async fn landing(serve_data: web::Data<Arc<ServeData>>) -> impl Responder {
    let mut subgraph_html = String::new();
    subgraph_html.push_str("<section class=\"supergraph-information\">");
    subgraph_html.push_str("<h3>Supergraph Status: Loaded ✅</h3>");
    subgraph_html.push_str(&format!(
        "<p><strong>Source: </strong> <i>{}</i></p>",
        serve_data.supergraph_source
    ));
    subgraph_html.push_str("<table>");
    subgraph_html.push_str("<tr><th>Subgraph</th><th>Transport</th><th>Location</th></tr>");
    for (subgraph_name, subgraph_endpoint) in &serve_data.subgraph_endpoint_map {
        subgraph_html.push_str("<tr>");
        subgraph_html.push_str(&format!("<td>{}</td>", subgraph_name));
        subgraph_html.push_str(&format!(
            "<td>{}</td>",
            if subgraph_endpoint.starts_with("http") {
                "http"
            } else {
                "Unknown"
            }
        ));
        subgraph_html.push_str(&format!(
            "<td><a href=\"{}\">{}</a></td>",
            subgraph_endpoint, subgraph_endpoint
        ));
        subgraph_html.push_str("</tr>");
    }
    subgraph_html.push_str("</table>");
    subgraph_html.push_str("</section>");
    Html::new(
        LANDING_PAGE_HTML
            .replace("__GRAPHIQL_LINK__", "/graphql")
            // TODO: Replace with actual path
            .replace("__REQUEST_PATH__", "/")
            .replace("__PRODUCT_NAME__", "Hive Gateway RS")
            .replace(
                "__PRODUCT_DESCRIPTION__",
                "A GraphQL Gateway written in Rust",
            )
            .replace("__PRODUCT_PACKAGE_NAME__", "hive-gateway-rs")
            .replace(
                "__PRODUCT_LINK__",
                "https://the-guild.dev/graphql/hive/docs/gateway",
            )
            .replace("__PRODUCT_LOGO__", __PRODUCT_LOGO__)
            .replace("__SUBGRAPH_HTML__", &subgraph_html),
    )
}

fn make_error_response(
    execution_result: ExecutionResult,
    accept_header: &Option<String>,
) -> HttpResponse {
    if accept_header
        .as_ref()
        .is_some_and(|header| header.contains("application/json"))
    {
        HttpResponse::Ok().json(execution_result)
    } else {
        HttpResponse::BadRequest().json(execution_result)
    }
}

async fn handle_execution_request(
    req: &HttpRequest,
    execution_request: &ExecutionRequest,
    serve_data: &Arc<ServeData>,
) -> HttpResponse {
    let accept_header = req
        .headers()
        .get("Accept")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    // TODO: Maybe cache here later
    let original_document = match safe_parse_operation(&execution_request.query) {
        Ok(doc) => doc,
        Err(err) => {
            return make_error_response(
                ExecutionResult {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: err.to_string(),
                        locations: None,
                        path: None,
                        extensions: Some(HashMap::from([(
                            "code".to_string(),
                            Value::String("BAD_REQUEST".to_string()),
                        )])),
                    }]),
                    extensions: None,
                },
                &accept_header,
            );
        }
    };

    debug!("original document: {}", original_document);

    let normalized_document = match normalize_operation(
        &serve_data.planner.consumer_schema,
        &original_document,
        execution_request.operation_name.as_deref(),
    ) {
        Ok(doc) => doc,
        Err(err) => {
            error!("Normalization error {err}");
            return make_error_response(
                ExecutionResult {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: "Unable to detect operation AST".to_string(),
                        locations: None,
                        extensions: Some(HashMap::from([(
                            "code".to_string(),
                            Value::String("BAD_REQUEST".to_string()),
                        )])),
                        path: None,
                    }]),
                    extensions: None,
                },
                &accept_header,
            );
        }
    };
    debug!("normalized document: {}", normalized_document);

    let operation = normalized_document.executable_operation();
    debug!("executable operation: {}", operation);

    if req.method() == Method::GET && operation.operation_kind.is_some() {
        if let Some(OperationKind::Mutation) = operation.operation_kind {
            return HttpResponse::MethodNotAllowed()
                .append_header(("allow", "POST"))
                .json(json!({
                    "errors": [
                        {
                            "message": "Cannot perform mutations over GET"
                        }
                    ]
                }));
        }
    }

    let consumer_schema_ast = &serve_data.planner.consumer_schema.document;
    let validation_cache_key = operation.hash();
    let validation_result = match serve_data.validate_cache.get(&validation_cache_key).await {
        Some(cached_validation) => cached_validation,
        None => {
            let validation_result = graphql_tools::validation::validate::validate(
                consumer_schema_ast,
                &original_document,
                &serve_data.validation_plan,
            );
            let validation_result_arc = Arc::new(validation_result);
            serve_data
                .validate_cache
                .insert(validation_cache_key, validation_result_arc.clone())
                .await;
            validation_result_arc
        }
    };

    if !validation_result.is_empty() {
        return make_error_response(
            ExecutionResult {
                data: None,
                errors: Some(
                    validation_result
                        .iter()
                        .map(|err| err.into())
                        .collect::<Vec<_>>(),
                ),
                extensions: None,
            },
            &accept_header,
        );
    }

    let (has_introspection, filtered_operation_for_plan) =
        query_planner::consumer_schema::introspection::filter_introspection_fields_in_operation(
            operation,
        );
    let plan_cache_key = filtered_operation_for_plan.hash();

    let query_plan = match serve_data.plan_cache.get(&plan_cache_key).await {
        Some(plan) => plan,
        None => {
            let query_plan =
                if filtered_operation_for_plan.selection_set.is_empty() && has_introspection {
                    query_planner::planner::plan_nodes::QueryPlan {
                        kind: "QueryPlan".to_string(),
                        node: None,
                    }
                } else {
                    match serve_data
                        .planner
                        .plan_from_normalized_operation(&filtered_operation_for_plan)
                    {
                        Ok(plan) => plan,
                        Err(err) => {
                            return make_error_response(
                                ExecutionResult {
                                    data: None,
                                    errors: Some(vec![GraphQLError {
                                        message: err.to_string(),
                                        locations: None,
                                        extensions: Some(HashMap::from([(
                                            "code".to_string(),
                                            Value::String("QUERY_PLAN_BUILD_FAILED".to_string()),
                                        )])),
                                        path: None,
                                    }]),
                                    extensions: None,
                                },
                                &accept_header,
                            );
                        }
                    }
                };
            let query_plan_arc = Arc::new(query_plan);
            serve_data
                .plan_cache
                .insert(plan_cache_key, query_plan_arc.clone())
                .await;
            query_plan_arc
        }
    };

    let variable_values = match query_plan_executor::variables::collect_variables(
        &filtered_operation_for_plan,
        &execution_request.variables,
        &serve_data.planner.consumer_schema.schema_metadata,
    ) {
        Ok(values) => values,
        Err(err) => {
            return make_error_response(
                ExecutionResult {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: err,
                        locations: None,
                        extensions: Some(HashMap::from([(
                            "code".to_string(),
                            Value::String("BAD_REQUEST".to_string()),
                        )])),
                        path: None,
                    }]),
                    extensions: None,
                },
                &accept_header,
            );
        }
    };

    let result = execute_query_plan(
        &query_plan,
        &serve_data.subgraph_endpoint_map,
        &variable_values,
        &serve_data.planner.consumer_schema.schema_metadata,
        operation,
        has_introspection,
        &serve_data.http_client,
    )
    .await;

    debug!("execution result: {:?}", result);

    let content_type: &str = if accept_header
        .as_ref()
        .is_some_and(|header| header.contains("application/graphql-response+json"))
    {
        "application/graphql-response+json"
    } else {
        "application/json"
    };

    HttpResponse::Ok()
        .content_type(content_type)
        .json(ExecutionResult {
            data: result.data,
            errors: result.errors,
            extensions: result.extensions,
        })
}

#[post("/graphql")]
async fn graphql_post(
    req: HttpRequest,
    request_body: web::Json<ExecutionRequest>,
    serve_data: web::Data<Arc<ServeData>>,
) -> impl Responder {
    handle_execution_request(&req, &request_body, &serve_data).await
}

static GRAPHILQL_HTML: &str = include_str!("../static/graphiql.html");

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct QueryParamsBody {
    query: Option<String>,
    operation_name: Option<String>,
    variables: Option<String>,
    extensions: Option<String>,
}

#[get("/graphql")]
async fn graphql_get(
    req: HttpRequest,
    params: web::Query<QueryParamsBody>,
    serve_data: web::Data<Arc<ServeData>>,
) -> impl Responder {
    let accept_header = req
        .headers()
        .get("Accept")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    if accept_header
        .as_ref()
        .is_some_and(|header| header.contains("text/html"))
    {
        HttpResponse::Ok()
            .content_type("text/html")
            .body(GRAPHILQL_HTML)
    } else {
        let query = match &params.query {
            Some(q) => q,
            None => {
                return make_error_response(
                    ExecutionResult {
                        data: None,
                        errors: Some(vec![GraphQLError {
                            message: "Query parameter is required".to_string(),
                            locations: None,
                            extensions: Some(HashMap::from([(
                                "code".to_string(),
                                Value::String("BAD_REQUEST".to_string()),
                            )])),
                            path: None,
                        }]),
                        extensions: None,
                    },
                    &accept_header,
                );
            }
        };
        let variables = match &params.variables {
            Some(v) => match serde_json::from_str::<HashMap<String, Value>>(v) {
                Ok(vars) => Some(vars),
                Err(err) => {
                    return make_error_response(
                        ExecutionResult {
                            data: None,
                            errors: Some(vec![GraphQLError {
                                message: err.to_string(),
                                locations: None,
                                extensions: Some(HashMap::from([(
                                    "code".to_string(),
                                    Value::String("BAD_REQUEST".to_string()),
                                )])),
                                path: None,
                            }]),
                            extensions: None,
                        },
                        &accept_header,
                    );
                }
            },
            None => None,
        };
        let extensions = match &params.extensions {
            Some(e) => match serde_json::from_str::<HashMap<String, Value>>(e) {
                Ok(exts) => Some(exts),
                Err(err) => {
                    return make_error_response(
                        ExecutionResult {
                            data: None,
                            errors: Some(vec![GraphQLError {
                                message: err.to_string(),
                                locations: None,
                                extensions: Some(HashMap::from([(
                                    "code".to_string(),
                                    Value::String("BAD_REQUEST".to_string()),
                                )])),
                                path: None,
                            }]),
                            extensions: None,
                        },
                        &accept_header,
                    );
                }
            },
            None => None,
        };
        let execution_request = ExecutionRequest {
            query: query.clone(),
            operation_name: params.operation_name.clone(),
            variables,
            extensions,
        };
        handle_execution_request(&req, &execution_request, &serve_data).await
    }
}
