use std::collections::HashMap;
use std::sync::Arc;
use std::{env, vec};

use actix_web::http::Method;
use actix_web::web::Html;
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use graphql_parser::schema::TypeDefinition;
use introspection::{
    filter_introspection_fields_in_operation, get_introspection_metadata,
    introspection_query_from_ast,
};
use query_plan_executor::ExecutionRequest;
use query_plan_executor::SchemaMetadata;
use query_plan_executor::{execute_query_plan, ExecutionResult};
use query_planner::ast::document::NormalizedDocument;
use query_planner::state::supergraph_state::{OperationKind, SupergraphState};
use query_planner::utils::parsing::parse_schema;
use query_planner::utils::parsing::safe_parse_operation;
use query_planner::{consumer_schema::ConsumerSchema, planner::Planner};
use serde_json::json;
use serde_json::Value::{self};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod builtin_types;
mod introspection;
mod value_from_ast;

#[actix_web::main]
async fn main() {
    let logger_enabled = env::var("DEBUG").is_ok();

    if logger_enabled {
        let tree_layer = tracing_tree::HierarchicalLayer::new(2)
            .with_bracketed_fields(true)
            .with_deferred_spans(false)
            .with_wraparound(25)
            .with_indent_lines(true)
            .with_timer(tracing_tree::time::Uptime::default())
            .with_thread_names(false)
            .with_thread_ids(false)
            .with_targets(false);

        tracing_subscriber::registry().with(tree_layer).init();
    }

    let args: Vec<String> = env::args().collect();

    let supergraph_path = &args[1];
    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let planner =
        Planner::new_from_supergraph_state(&supergraph_state).expect("failed to create planner");
    let schema_metadata = planner.consumer_schema().schema_metadata();
    let serve_data = ServeData {
        supergraph_source: supergraph_path.to_string(),
        planner,
        schema_metadata,
        subgraph_endpoint_map: supergraph_state.subgraph_endpoint_map,
        http_client: reqwest::Client::new(),
        plan_cache: moka::future::Cache::new(1000),
        parse_cache: moka::future::Cache::new(1000),
        filtered_operation_cache: moka::future::Cache::new(1000),
    };
    let serve_data_arc = Arc::new(serve_data);
    println!("Starting server on http://localhost:4000");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(serve_data_arc.clone()))
            .service(graphiql)
            .service(graphql_endpoint)
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
    schema_metadata: SchemaMetadata,
    planner: Planner,
    subgraph_endpoint_map: HashMap<String, String>,
    http_client: reqwest::Client,
    plan_cache: moka::future::Cache<u64, Arc<query_planner::planner::plan_nodes::QueryPlan>>,
    parse_cache: moka::future::Cache<String, Arc<NormalizedDocument>>,
    filtered_operation_cache: moka::future::Cache<
        String,
        Arc<(bool, query_planner::ast::operation::OperationDefinition)>,
    >,
}

fn collect_variables(
    operation: &query_planner::ast::operation::OperationDefinition,
    variables: &Option<HashMap<String, Value>>,
) -> Result<Option<HashMap<String, Value>>, String> {
    if operation.variable_definitions.is_none() {
        return Ok(None);
    }
    let variable_definitions = operation.variable_definitions.as_ref().unwrap();
    let collected_variables: Result<Vec<Option<(String, Value)>>, String> = variable_definitions
        .iter()
        .map(|variable_definition| {
            let variable_name = variable_definition.name.to_string();
            if let Some(variable_value) = variables.as_ref().and_then(|v| v.get(&variable_name)) {
                if variable_value.is_null() && variable_definition.variable_type.is_non_null() {
                    return Err(format!(
                        "Variable '{}' is non-nullable but no value was provided",
                        variable_name
                    ));
                }
                return Ok(Some((variable_name, variable_value.clone())));
            }
            if let Some(default_value) = &variable_definition.default_value {
                // Assuming value_from_ast now returns Result<Value, String> or similar
                // and needs to be adapted if it returns Option or panics.
                // For now, let's assume it can return an Err that needs to be propagated.
                let default_value_coerced: Value = default_value.into();
                if !default_value_coerced.is_null() {
                    return Ok(Some((variable_name, default_value_coerced)));
                }
            }
            if variable_definition.variable_type.is_non_null() {
                return Err(format!(
                    "Variable '{}' is non-nullable but no value was provided",
                    variable_name
                ));
            }
            Ok(None)
        })
        .collect();

    let variable_values: HashMap<String, Value> =
        collected_variables?.into_iter().flatten().collect();

    if variable_values.is_empty() {
        Ok(None)
    } else {
        Ok(Some(variable_values))
    }
}

fn get_type_name_of_ast(type_ast: &graphql_parser::schema::Type<'static, String>) -> String {
    match type_ast {
        graphql_parser::schema::Type::NamedType(named_type) => named_type.to_string(),
        graphql_parser::schema::Type::NonNullType(non_null_type) => {
            get_type_name_of_ast(non_null_type)
        }
        graphql_parser::schema::Type::ListType(list_type) => get_type_name_of_ast(list_type),
    }
}
trait SchemaWithMetadata {
    fn schema_metadata(&self) -> SchemaMetadata;
}

impl SchemaWithMetadata for ConsumerSchema {
    fn schema_metadata(&self) -> SchemaMetadata {
        let mut first_possible_types: HashMap<String, Vec<String>> = HashMap::new();
        let introspection_metadata = get_introspection_metadata();
        let mut type_fields = introspection_metadata.type_fields;
        let mut enum_values = introspection_metadata.enum_values;
        for definition in &self.document.definitions {
            match definition {
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(
                    enum_type,
                )) => {
                    let name = enum_type.name.to_string();
                    let mut values = vec![];
                    for enum_value in &enum_type.values {
                        values.push(enum_value.name.to_string());
                    }
                    enum_values.insert(name, values);
                }
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                    object_type,
                )) => {
                    let name = object_type.name.to_string();
                    let fields = type_fields.entry(name).or_default();
                    for field in &object_type.fields {
                        let field_type_name = get_type_name_of_ast(&field.field_type);
                        fields.insert(field.name.to_string(), field_type_name);
                    }

                    for interface in &object_type.implements_interfaces {
                        let interface_name = interface.to_string();
                        let possible_types_entry =
                            first_possible_types.entry(interface_name).or_default();
                        possible_types_entry.push(object_type.name.to_string());
                    }
                }
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                    interface_type,
                )) => {
                    let name = interface_type.name.to_string();
                    let mut fields = HashMap::new();
                    for field in &interface_type.fields {
                        let field_type_name = get_type_name_of_ast(&field.field_type);
                        fields.insert(field.name.to_string(), field_type_name);
                    }
                    type_fields.insert(name, fields);
                    for interface_name in &interface_type.implements_interfaces {
                        let interface_name = interface_name.to_string();
                        let possible_types_entry =
                            first_possible_types.entry(interface_name).or_default();
                        possible_types_entry.push(interface_type.name.to_string());
                    }
                }
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                    union_type,
                )) => {
                    let name = union_type.name.to_string();
                    let mut types = vec![];
                    for member in &union_type.types {
                        types.push(member.to_string());
                    }
                    first_possible_types.insert(name, types);
                }
                _ => {}
            }
        }
        let mut final_possible_types: HashMap<String, Vec<String>> = HashMap::new();
        // Re-iterate over the possible_types
        for (definition_name_of_x, first_possible_types_of_x) in &first_possible_types {
            let mut possible_types_of_x: Vec<String> = Vec::new();
            for definition_name_of_y in first_possible_types_of_x {
                possible_types_of_x.push(definition_name_of_y.to_string());
                let possible_types_of_y = first_possible_types.get(definition_name_of_y);
                if let Some(possible_types_of_y) = possible_types_of_y {
                    for definition_name_of_z in possible_types_of_y {
                        possible_types_of_x.push(definition_name_of_z.to_string());
                    }
                }
            }
            final_possible_types.insert(definition_name_of_x.to_string(), possible_types_of_x);
        }
        let introspection_query = introspection_query_from_ast(&self.document);
        let introspection_schema_root_json = json!(introspection_query.__schema);
        SchemaMetadata {
            possible_types: final_possible_types,
            enum_values,
            type_fields,
            introspection_schema_root_json,
        }
    }
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
    message: &str,
    accept_header: &Option<String>,
    is_graphql_error: bool,
    error_code: &str,
) -> HttpResponse {
    if !is_graphql_error {
        HttpResponse::BadRequest().json(json!({
            "errors": [
                {
                    "message": message,
                    "extensions": {
                        "code": error_code
                    }
                }
            ]
        }))
    } else if accept_header
        .as_ref()
        .is_some_and(|header| header.contains("application/json"))
    {
        HttpResponse::Ok().json(json!({
            "errors": [
                {
                    "message": message,
                    "extensions": {
                        "code": error_code
                    }
                }
            ]
        }))
    } else {
        HttpResponse::BadRequest().json(json!({
            "errors": [
                {
                    "message": message,
                    "extensions": {
                        "code": error_code
                    }
                }
            ]
        }))
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
    let query_and_operation_name = format!(
        "{}_{}",
        execution_request.query,
        execution_request.operation_name.clone().unwrap_or_default()
    );

    let normalized_document = match serve_data.parse_cache.get(&query_and_operation_name).await {
        Some(normalized_document) => normalized_document,
        None => {
            let doc = match safe_parse_operation(&execution_request.query) {
                Ok(doc) => doc,
                Err(err) => {
                    return make_error_response(
                        &err.to_string(),
                        &accept_header,
                        true,
                        "BAD_REQUEST",
                    );
                }
            };
            let normalized_document =
                Arc::new(query_planner::utils::operation_utils::prepare_document(
                    doc,
                    execution_request.operation_name.as_deref(),
                ));
            serve_data
                .parse_cache
                .insert(execution_request.query.clone(), normalized_document.clone())
                .await;
            normalized_document
        }
    };

    let filtered_op_data = match serve_data
        .filtered_operation_cache
        .get(&query_and_operation_name)
        .await
    {
        Some(filtered_op_data) => filtered_op_data,
        None => {
            let operation = match normalized_document.executable_operation() {
                Some(operation) => operation,
                None => {
                    return make_error_response(
                        "Unable to detect operation AST",
                        &accept_header,
                        true,
                        "BAD_REQUEST",
                    );
                }
            };
            let filtered_op_data = filter_introspection_fields_in_operation(operation);
            let filtered_op_arc = Arc::new(filtered_op_data);
            serve_data
                .filtered_operation_cache
                .insert(query_and_operation_name.clone(), filtered_op_arc.clone())
                .await;
            filtered_op_arc
        }
    };

    let has_introspection = filtered_op_data.0;
    let filtered_operation_for_plan = &filtered_op_data.1;

    if req.method() == Method::GET && filtered_operation_for_plan.operation_kind.is_some() {
        if let Some(OperationKind::Mutation) = filtered_operation_for_plan.operation_kind {
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
                        .plan_from_normalized_operation(filtered_operation_for_plan)
                    {
                        Ok(plan) => plan,
                        Err(err) => {
                            return make_error_response(
                                &err.to_string(),
                                &accept_header,
                                true,
                                "QUERY_PLANNER_ERROR",
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

    let variable_values =
        match collect_variables(filtered_operation_for_plan, &execution_request.variables) {
            Ok(values) => values,
            Err(err) => {
                return make_error_response(&err, &accept_header, true, "BAD_REQUEST");
            }
        };
    let result = execute_query_plan(
        &query_plan,
        &serve_data.subgraph_endpoint_map,
        &variable_values,
        &serve_data.schema_metadata,
        &normalized_document,
        has_introspection,
        &serve_data.http_client,
    )
    .await;

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
async fn graphql_endpoint(
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
async fn graphiql(
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
        if params.query.is_none() {
            return make_error_response(
                "Missing query parameter",
                &accept_header,
                true,
                "BAD_REQUEST",
            );
        }
        let variables = params
            .variables
            .as_ref()
            .map(|v| serde_json::from_str::<HashMap<String, Value>>(v));
        let extensions = params
            .extensions
            .as_ref()
            .map(|e| serde_json::from_str::<HashMap<String, Value>>(e));
        if let Some(Err(err)) = variables {
            return make_error_response(&err.to_string(), &accept_header, true, "BAD_REQUEST");
        }
        if let Some(Err(err)) = extensions {
            return make_error_response(&err.to_string(), &accept_header, true, "BAD_REQUEST");
        }
        let query = params.query.clone().unwrap();
        let execution_request = ExecutionRequest {
            query,
            operation_name: params.operation_name.clone(),
            variables: variables.and_then(|v| v.ok()),
            extensions: extensions.and_then(|v| v.ok()),
        };
        handle_execution_request(&req, &execution_request, &serve_data).await
    }
}
