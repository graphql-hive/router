use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use actix_web::web::Html;
use actix_web::{get, post, web, App, HttpServer, Responder};
use graphql_parser::query::OperationDefinition;
use graphql_parser::schema::TypeDefinition;
use query_plan_executor::execute_query_plan;
use query_plan_executor::ExecutionRequest;
use query_plan_executor::SchemaMetadata;
use query_planner::consumer_schema::ConsumerSchema;
use query_planner::planner::Planner;
use query_planner::state::supergraph_state::SupergraphState;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
use serde_json::json;
use serde_json::Value::{self};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
        planner,
        schema_metadata,
        subgraph_endpoint_map: supergraph_state.subgraph_endpoint_map,
    };
    let serve_data_arc = Arc::new(serve_data);
    println!("Starting server on http://localhost:4000");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(serve_data_arc.clone()))
            .service(graphiql)
            .service(graphql_endpoint)
    })
    .bind(("127.0.0.1", 4000))
    .expect("Failed to bind server")
    .run()
    .await
    .expect("Failed to run server")
}

struct ServeData {
    schema_metadata: SchemaMetadata,
    planner: Planner,
    subgraph_endpoint_map: HashMap<String, String>,
}

fn from_graphql_value_to_serde_value(
    value: &graphql_parser::query::Value<'static, String>,
    variables: &Option<HashMap<String, Value>>,
) -> serde_json::Value {
    match value {
        graphql_parser::query::Value::Null => serde_json::Value::Null,
        graphql_parser::query::Value::Boolean(b) => serde_json::Value::Bool(*b),
        graphql_parser::query::Value::String(s) => serde_json::Value::String(s.to_string()),
        graphql_parser::query::Value::Enum(e) => serde_json::Value::String(e.to_string()),
        // TODO: Handle variable parsing errors here just like in GraphQL-JS
        graphql_parser::query::Value::Int(n) => serde_json::Value::Number(
            serde_json::Number::from(n.as_i64().expect("Failed to coerce")),
        ),
        graphql_parser::query::Value::Float(n) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*n).expect("Failed to coerce"))
        }
        graphql_parser::query::Value::List(l) => serde_json::Value::Array(
            l.iter()
                .map(|v| from_graphql_value_to_serde_value(v, variables))
                .collect(),
        ),
        graphql_parser::query::Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        from_graphql_value_to_serde_value(v, variables),
                    )
                })
                .collect(),
        ),
        graphql_parser::query::Value::Variable(var_name) => {
            if let Some(variables_map) = variables {
                if let Some(value) = variables_map.get(var_name) {
                    value.clone() // Return the value from the variables map
                } else {
                    serde_json::Value::Null // If variable not found, return null
                }
            } else {
                serde_json::Value::Null // If no variables provided, return null
            }
        }
    }
}

fn collect_variables(
    operation: &OperationDefinition<'static, String>,
    variables: &Option<HashMap<String, Value>>,
) -> Option<HashMap<String, Value>> {
    let variable_definitions = match &operation {
        OperationDefinition::SelectionSet(_) => return None,
        OperationDefinition::Query(query) => &query.variable_definitions,
        OperationDefinition::Mutation(mutation) => &mutation.variable_definitions,
        OperationDefinition::Subscription(subscription) => &subscription.variable_definitions,
    };
    let variable_values: HashMap<String, Value> = variable_definitions
        .iter()
        .filter_map(|variable_definition| {
            let variable_name = variable_definition.name.to_string();
            if let Some(variable_value) = variables.as_ref().and_then(|v| v.get(&variable_name)) {
                return Some((variable_name, variable_value.clone()));
            }
            if let Some(default_value) = &variable_definition.default_value {
                let default_value_coerced =
                    from_graphql_value_to_serde_value(default_value, variables);
                if !default_value_coerced.is_null() {
                    return Some((variable_name, default_value_coerced));
                }
            }
            None
        })
        .collect();
    if variable_values.is_empty() {
        None
    } else {
        Some(variable_values)
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
        let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();
        let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
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
                    let mut fields = HashMap::new();
                    for field in &object_type.fields {
                        let field_type_name = get_type_name_of_ast(&field.field_type);
                        fields.insert(field.name.to_string(), field_type_name);
                    }
                    type_fields.insert(name, fields);

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
        SchemaMetadata {
            possible_types: final_possible_types,
            enum_values,
            type_fields,
        }
    }
}

static GRAPHILQL_HTML: &str = include_str!("../static/graphiql.html");

#[get("/graphql")]
async fn graphiql() -> impl Responder {
    Html::new(GRAPHILQL_HTML)
}

#[post("/graphql")]
async fn graphql_endpoint(
    request_body: web::Json<ExecutionRequest>,
    serve_data: web::Data<ServeData>,
) -> impl Responder {
    let query_str = request_body.query.as_deref().expect("query is required");
    let operation_name = request_body.operation_name.as_deref();
    let document = parse_operation(query_str);

    let query_plan = serve_data
        .planner
        .plan(&document, operation_name)
        .expect("failed to build query plan");

    // TODO: Fix that, it should really be handled differently
    let operation = match &document.definitions[0] {
        graphql_parser::query::Definition::Operation(operation) => operation,
        _ => panic!("Expected an operation definition"),
    };

    let variable_values = collect_variables(operation, &request_body.variables);
    let mut result = execute_query_plan(
        &query_plan,
        &serve_data.subgraph_endpoint_map,
        &variable_values,
        &serve_data.schema_metadata,
        &document,
    )
    .await;

    let mut extensions = HashMap::new();
    extensions.insert("queryPlan".to_string(), json!(query_plan));
    result.extensions = Some(extensions);
    web::Json(result)
}
