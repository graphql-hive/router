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
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
use serde_json::json;
use serde_json::Value;
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
    let planner = Planner::new_from_supergraph(&parsed_schema).expect("failed to create planner");
    let schema_metadata = planner.consumer_schema().schema_metadata();
    let subgraph_endpoint_map = get_subgraph_endpoint_map(&parsed_schema);
    let serve_data = ServeData {
        planner,
        subgraph_endpoint_map,
        schema_metadata,
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
    subgraph_endpoint_map: HashMap<String, String>,
    schema_metadata: SchemaMetadata,
    planner: Planner,
}

fn collect_variables(
    operation: &OperationDefinition<'static, String>,
    variables: &Option<HashMap<String, Value>>,
) -> Option<HashMap<String, Value>> {
    let mut variable_values = HashMap::new();
    let variable_definitions = match &operation {
        OperationDefinition::SelectionSet(_) => &vec![],
        OperationDefinition::Query(query) => &query.variable_definitions,
        OperationDefinition::Mutation(mutation) => &mutation.variable_definitions,
        OperationDefinition::Subscription(subscription) => &subscription.variable_definitions,
    };
    for variable_definition in variable_definitions {
        let variable_name = variable_definition.name.to_string();
        if let Some(variable_value) = variables.as_ref().and_then(|v| v.get(&variable_name)) {
            variable_values.insert(variable_name, variable_value.clone());
        } else if let Some(default_value) = &variable_definition.default_value {
            todo!("Not implemented yet: default value: {}", default_value);
        } else {
            variable_values.insert(variable_name, Value::Null);
        }
    }
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
            get_type_name_of_ast(&non_null_type)
        }
        graphql_parser::schema::Type::ListType(list_type) => get_type_name_of_ast(&list_type),
    }
}

fn get_subgraph_endpoint_map(
    supergraph_ast: &graphql_parser::schema::Document<'static, String>,
) -> HashMap<String, String> {
    let mut subgraph_endpoint_map = HashMap::new();
    for definition in &supergraph_ast.definitions {
        match definition {
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                let name = enum_type.name.to_string();
                if name == "join__Graph" {
                    for enum_value in &enum_type.values {
                        let directive = enum_value
                            .directives
                            .iter()
                            .find(|d| d.name == "join__graph");
                        if let Some(directive) = directive {
                            let mut subgraph_name = "".to_string();
                            let mut endpoint = "".to_string();
                            for (argument_name, argument_value) in &directive.arguments {
                                if argument_name == "name" {
                                    match argument_value {
                                        graphql_parser::schema::Value::String(enum_value) => {
                                            subgraph_name = enum_value.to_string();
                                        }
                                        _ => {
                                            panic!("Expected enum value for name");
                                        }
                                    }
                                } else if argument_name == "url" {
                                    match argument_value {
                                        graphql_parser::schema::Value::String(enum_value) => {
                                            endpoint = enum_value.to_string();
                                        }
                                        _ => {
                                            panic!("Expected enum value for url");
                                        }
                                    }
                                }
                            }
                            if !subgraph_name.is_empty() && !endpoint.is_empty() {
                                subgraph_endpoint_map.insert(subgraph_name, endpoint);
                            }
                        }
                    }
                }
                break;
            }
            _ => {}
        }
    }

    subgraph_endpoint_map
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
        for (definition_name_of_x, first_possible_types_of_x) in first_possible_types.iter() {
            let mut possible_types_of_x: Vec<String> = Vec::new();
            for definition_name_of_y in first_possible_types_of_x.iter() {
                possible_types_of_x.push(definition_name_of_y.to_string());
                let possible_types_of_y = first_possible_types.get(&definition_name_of_y.clone());
                if let Some(possible_types_of_y) = possible_types_of_y {
                    for definition_name_of_z in possible_types_of_y.iter() {
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
