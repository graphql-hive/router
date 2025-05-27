use std::collections::HashMap;
use std::sync::Arc;
use std::{env, vec};

use actix_web::web::Html;
use actix_web::{get, post, web, App, HttpServer, Responder};
use graphql_parser::query::OperationDefinition;
use graphql_parser::schema::TypeDefinition;
use graphql_parser::Pos;
use graphql_tools::introspection::{
    IntrospectionDirective, IntrospectionEnumType, IntrospectionEnumValue, IntrospectionField,
    IntrospectionInputObjectType, IntrospectionInputTypeRef, IntrospectionInputValue,
    IntrospectionInterfaceType, IntrospectionNamedTypeRef, IntrospectionObjectType,
    IntrospectionOutputTypeRef, IntrospectionQuery, IntrospectionScalarType, IntrospectionSchema,
    IntrospectionType, IntrospectionUnionType,
};
use query_plan_executor::ExecutionRequest;
use query_plan_executor::SchemaMetadata;
use query_plan_executor::{execute_query_plan, ExecutionResult};
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
            .default_service(web::route().to(landing))
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

static LANDING_PAGE_HTML: &str = include_str!("../static/landing-page.html");
static __PRODUCT_LOGO__: &str = include_str!("../static/product_logo.svg");

async fn landing(serve_data: web::Data<Arc<ServeData>>) -> impl Responder {
    let mut subgraph_html = String::new();
    subgraph_html.push_str("<section class=\"supergraph-information\">");
    subgraph_html.push_str("<h3>Supergraph Status: Loaded ✅</h3>");
    subgraph_html.push_str("<p><strong>Source: </strong> <i>supergraph.graphql</i></p>");
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

static GRAPHILQL_HTML: &str = include_str!("../static/graphiql.html");

#[get("/graphql")]
async fn graphiql() -> impl Responder {
    Html::new(GRAPHILQL_HTML)
}

#[post("/graphql")]
async fn graphql_endpoint(
    request_body: web::Json<ExecutionRequest>,
    serve_data: web::Data<Arc<ServeData>>,
) -> impl Responder {
    let operation_name = request_body.operation_name.as_deref();
    let query_str = request_body.query.as_deref().expect("query is required");
    let document = parse_operation(query_str);

    if operation_name == Some("IntrospectionQuery") {
        let consumer_schema = serve_data.planner.consumer_schema();
        let introspection_query = introspection_query_from_ast(&consumer_schema.document);
        return web::Json(ExecutionResult {
            data: Some(json!(introspection_query)),
            errors: None,
            extensions: None,
        });
    }

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

fn introspection_output_type_ref_from_ast(
    ast: &graphql_parser::schema::Type<'static, String>,
    type_ast_map: &HashMap<String, graphql_parser::schema::Definition<'static, String>>,
) -> IntrospectionOutputTypeRef {
    match ast {
        graphql_parser::schema::Type::ListType(of_type) => IntrospectionOutputTypeRef::LIST {
            of_type: Some(Box::new(introspection_output_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NonNullType(of_type) => {
            IntrospectionOutputTypeRef::NON_NULL {
                of_type: Some(Box::new(introspection_output_type_ref_from_ast(
                    of_type,
                    type_ast_map,
                ))),
            }
        }
        graphql_parser::schema::Type::NamedType(named_type) => {
            let named_type_definition = type_ast_map
                .get(named_type)
                .unwrap_or_else(|| panic!("Type {} not found in type AST map", named_type));
            match named_type_definition {
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                    scalar_type,
                )) => IntrospectionOutputTypeRef::SCALAR(IntrospectionNamedTypeRef {
                    name: scalar_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                    object_type,
                )) => IntrospectionOutputTypeRef::OBJECT(IntrospectionNamedTypeRef {
                    name: object_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                    interface_type,
                )) => IntrospectionOutputTypeRef::INTERFACE(IntrospectionNamedTypeRef {
                    name: interface_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                    union_type,
                )) => IntrospectionOutputTypeRef::UNION(IntrospectionNamedTypeRef {
                    name: union_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(
                    enum_type,
                )) => IntrospectionOutputTypeRef::ENUM(IntrospectionNamedTypeRef {
                    name: enum_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(
                    TypeDefinition::InputObject(input_object_type),
                ) => IntrospectionOutputTypeRef::INPUT_OBJECT(IntrospectionNamedTypeRef {
                    name: input_object_type.name.to_string(),
                }),
                _ => panic!("Unsupported type definition for introspection"),
            }
        }
    }
}

fn introspection_input_type_ref_from_ast(
    ast: &graphql_parser::schema::Type<'static, String>,
    type_ast_map: &HashMap<String, graphql_parser::schema::Definition<'static, String>>,
) -> IntrospectionInputTypeRef {
    match ast {
        graphql_parser::schema::Type::ListType(of_type) => IntrospectionInputTypeRef::LIST {
            of_type: Some(Box::new(introspection_input_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NonNullType(of_type) => IntrospectionInputTypeRef::NON_NULL {
            of_type: Some(Box::new(introspection_input_type_ref_from_ast(
                of_type,
                type_ast_map,
            ))),
        },
        graphql_parser::schema::Type::NamedType(named_type) => {
            let named_type_definition = type_ast_map
                .get(named_type)
                .unwrap_or_else(|| panic!("Type {} not found in type AST map", named_type));
            match named_type_definition {
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                    scalar_type,
                )) => IntrospectionInputTypeRef::SCALAR(IntrospectionNamedTypeRef {
                    name: scalar_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(
                    enum_type,
                )) => IntrospectionInputTypeRef::ENUM(IntrospectionNamedTypeRef {
                    name: enum_type.name.to_string(),
                }),
                graphql_parser::schema::Definition::TypeDefinition(
                    TypeDefinition::InputObject(input_object_type),
                ) => IntrospectionInputTypeRef::INPUT_OBJECT(IntrospectionNamedTypeRef {
                    name: input_object_type.name.to_string(),
                }),
                _ => panic!("Unsupported type definition for introspection"),
            }
        }
    }
}

fn introspection_query_from_ast(
    ast: &graphql_parser::schema::Document<'static, String>,
) -> IntrospectionQuery {
    // Add known scalar types to the type AST map
    let mut type_ast_map: HashMap<String, graphql_parser::schema::Definition<'static, String>> =
        ["String", "Int", "Float", "Boolean", "ID"]
            .iter()
            .map(|scalar_type| {
                (
                    scalar_type.to_string(),
                    graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                        graphql_parser::schema::ScalarType {
                            name: scalar_type.to_string(),
                            description: None,
                            position: Pos::default(),
                            directives: vec![],
                        },
                    )),
                )
            })
            .collect();
    let mut schema_definition: Option<&graphql_parser::schema::SchemaDefinition<'static, String>> =
        None;
    for definition in &ast.definitions {
        let type_name = match &definition {
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                scalar_type,
            )) => Some(&scalar_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                object_type,
            )) => Some(&object_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                interface_type,
            )) => Some(&interface_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                union_type,
            )) => Some(&union_type.name),
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                Some(&enum_type.name)
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::InputObject(
                input_object_type,
            )) => Some(&input_object_type.name),
            graphql_parser::schema::Definition::DirectiveDefinition(directive) => {
                Some(&directive.name)
            }
            graphql_parser::schema::Definition::SchemaDefinition(schema_definition_ast) => {
                schema_definition = Some(schema_definition_ast);
                None
            }
            _ => None,
        };
        if let Some(type_name) = type_name {
            type_ast_map.insert(type_name.clone(), definition.clone());
        }
    }

    let mut types = vec![];
    let mut directives = vec![];

    for definition in type_ast_map.values() {
        match definition {
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Scalar(
                scalar_type,
            )) => {
                types.push(IntrospectionType::SCALAR(IntrospectionScalarType {
                    name: scalar_type.name.to_string(),
                    description: scalar_type.description.clone(),
                    // TODO: specified_by_url is missing in graphql_parser::schema::ScalarType
                    specified_by_url: None,
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Object(
                object_type,
            )) => {
                types.push(IntrospectionType::OBJECT(IntrospectionObjectType {
                    name: object_type.name.to_string(),
                    description: object_type.description.clone(),
                    fields: object_type
                        .fields
                        .iter()
                        .map(|field| {
                            IntrospectionField {
                                name: field.name.to_string(),
                                description: field.description.clone(),
                                // TODO: Handle deprecation
                                is_deprecated: None,
                                deprecation_reason: None,
                                args: field
                                    .arguments
                                    .iter()
                                    .map(|arg| IntrospectionInputValue {
                                        name: arg.name.to_string(),
                                        description: arg.description.clone(),
                                        type_ref: Some(introspection_input_type_ref_from_ast(
                                            &arg.value_type,
                                            &type_ast_map,
                                        )),
                                        default_value: None, // TODO: Handle default values
                                        is_deprecated: None, // TODO: Handle deprecation
                                        deprecation_reason: None, // TODO: Handle deprecation reason
                                    })
                                    .collect(),
                                type_ref: introspection_output_type_ref_from_ast(
                                    &field.field_type,
                                    &type_ast_map,
                                ),
                            }
                        })
                        .collect(),
                    interfaces: object_type
                        .implements_interfaces
                        .iter()
                        .map(|i| IntrospectionNamedTypeRef {
                            name: i.to_string(),
                        })
                        .collect(),
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Interface(
                interface_type,
            )) => {
                types.push(IntrospectionType::INTERFACE(IntrospectionInterfaceType {
                    name: interface_type.name.to_string(),
                    description: interface_type.description.clone(),
                    fields: interface_type
                        .fields
                        .iter()
                        .map(|field| {
                            IntrospectionField {
                                name: field.name.to_string(),
                                description: field.description.clone(),
                                // TODO: Handle deprecation
                                is_deprecated: None,
                                deprecation_reason: None,
                                args: field
                                    .arguments
                                    .iter()
                                    .map(|arg| IntrospectionInputValue {
                                        name: arg.name.to_string(),
                                        description: arg.description.clone(),
                                        type_ref: Some(introspection_input_type_ref_from_ast(
                                            &arg.value_type,
                                            &type_ast_map,
                                        )),
                                        default_value: None, // TODO: Handle default values
                                        is_deprecated: None, // TODO: Handle deprecation
                                        deprecation_reason: None, // TODO: Handle deprecation reason
                                    })
                                    .collect(),
                                type_ref: introspection_output_type_ref_from_ast(
                                    &field.field_type,
                                    &type_ast_map,
                                ),
                            }
                        })
                        .collect(),
                    interfaces: Some(
                        interface_type
                            .implements_interfaces
                            .iter()
                            .map(|i| IntrospectionNamedTypeRef {
                                name: i.to_string(),
                            })
                            .collect(),
                    ),
                    // TODO: Handle possible types
                    possible_types: vec![],
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Union(
                union_type,
            )) => {
                types.push(IntrospectionType::UNION(IntrospectionUnionType {
                    name: union_type.name.to_string(),
                    description: union_type.description.clone(),
                    possible_types: union_type
                        .types
                        .iter()
                        .map(|t| IntrospectionNamedTypeRef {
                            name: t.to_string(),
                        })
                        .collect(),
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::Enum(enum_type)) => {
                types.push(IntrospectionType::ENUM(IntrospectionEnumType {
                    name: enum_type.name.to_string(),
                    description: enum_type.description.clone(),
                    enum_values: enum_type
                        .values
                        .iter()
                        .map(|enum_value| IntrospectionEnumValue {
                            name: enum_value.name.to_string(),
                            description: None,        // TODO: Handle description
                            is_deprecated: None,      // TODO: Handle deprecation
                            deprecation_reason: None, // TODO: Handle deprecation reason
                        })
                        .collect(),
                }));
            }
            graphql_parser::schema::Definition::TypeDefinition(TypeDefinition::InputObject(
                input_object_type,
            )) => {
                types.push(IntrospectionType::INPUT_OBJECT(
                    IntrospectionInputObjectType {
                        name: input_object_type.name.to_string(),
                        description: input_object_type.description.clone(),
                        input_fields: input_object_type
                            .fields
                            .iter()
                            .map(|field| {
                                IntrospectionInputValue {
                                    name: field.name.to_string(),
                                    description: field.description.clone(),
                                    type_ref: Some(introspection_input_type_ref_from_ast(
                                        &field.value_type,
                                        &type_ast_map,
                                    )),
                                    // TODO: Handle default values
                                    default_value: None,
                                    // TODO: Handle deprecation
                                    is_deprecated: None,
                                    deprecation_reason: None,
                                }
                            })
                            .collect(),
                    },
                ));
            }
            graphql_parser::schema::Definition::DirectiveDefinition(directive) => {
                directives.push(IntrospectionDirective {
                    name: directive.name.to_string(),
                    description: directive.description.clone(),
                    locations: directive
                        .locations
                        .iter()
                        .map(|l| {
                            match l {
                                graphql_parser::schema::DirectiveLocation::Query => graphql_tools::introspection::DirectiveLocation::QUERY,
                                graphql_parser::schema::DirectiveLocation::Mutation => graphql_tools::introspection::DirectiveLocation::MUTATION,
                                graphql_parser::schema::DirectiveLocation::Subscription => graphql_tools::introspection::DirectiveLocation::SUBSCRIPTION,
                                graphql_parser::schema::DirectiveLocation::Field => graphql_tools::introspection::DirectiveLocation::FIELD,
                                graphql_parser::schema::DirectiveLocation::FragmentDefinition => graphql_tools::introspection::DirectiveLocation::FRAGMENT_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::FragmentSpread => graphql_tools::introspection::DirectiveLocation::FRAGMENT_SPREAD,
                                graphql_parser::schema::DirectiveLocation::InlineFragment => graphql_tools::introspection::DirectiveLocation::INLINE_FRAGMENT,
                                graphql_parser::schema::DirectiveLocation::VariableDefinition => graphql_tools::introspection::DirectiveLocation::VARIABLE_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::Schema => graphql_tools::introspection::DirectiveLocation::SCHEMA,
                                graphql_parser::schema::DirectiveLocation::Scalar => graphql_tools::introspection::DirectiveLocation::SCALAR,
                                graphql_parser::schema::DirectiveLocation::Object => graphql_tools::introspection::DirectiveLocation::OBJECT,
                                graphql_parser::schema::DirectiveLocation::FieldDefinition => graphql_tools::introspection::DirectiveLocation::FIELD_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::ArgumentDefinition => graphql_tools::introspection::DirectiveLocation::ARGUMENT_DEFINITION,
                                graphql_parser::schema::DirectiveLocation::Interface => graphql_tools::introspection::DirectiveLocation::INTERFACE,
                                graphql_parser::schema::DirectiveLocation::Union => graphql_tools::introspection::DirectiveLocation::UNION,
                                graphql_parser::schema::DirectiveLocation::Enum => graphql_tools::introspection::DirectiveLocation::ENUM,
                                graphql_parser::schema::DirectiveLocation::EnumValue => graphql_tools::introspection::DirectiveLocation::ENUM_VALUE,
                                graphql_parser::schema::DirectiveLocation::InputObject => graphql_tools::introspection::DirectiveLocation::INPUT_OBJECT,
                                graphql_parser::schema::DirectiveLocation::InputFieldDefinition => graphql_tools::introspection::DirectiveLocation::INPUT_FIELD_DEFINITION,
                            }
                        })
                        .collect(),
                    is_repeatable: Some(directive.repeatable),
                    args: directive
                        .arguments
                        .iter()
                        .map(|arg| IntrospectionInputValue {
                            name: arg.name.to_string(),
                            description: arg.description.clone(),
                            type_ref: Some(introspection_input_type_ref_from_ast(&arg.value_type, &type_ast_map)),
                            default_value: None, // TODO: Handle default values
                            is_deprecated: None, // TODO: Handle deprecation
                            deprecation_reason: None, // TODO: Handle deprecation reason
                        })
                        .collect(),
                });
            }
            _ => {
                // Ignore other definitions like TypeExtension, SchemaDefinition, etc.
            }
        }
    }

    IntrospectionQuery {
        __schema: IntrospectionSchema {
            query_type: IntrospectionNamedTypeRef {
                name: schema_definition
                    .as_ref()
                    .and_then(|sd| sd.query.as_ref())
                    .map_or("Query".to_string(), |qt| qt.to_string()),
            },
            mutation_type: schema_definition
                .as_ref()
                .and_then(|sd| sd.mutation.as_ref())
                .map(|mt| IntrospectionNamedTypeRef {
                    name: mt.to_string(),
                }),
            subscription_type: schema_definition
                .as_ref()
                .and_then(|sd| sd.subscription.as_ref())
                .map(|st| IntrospectionNamedTypeRef {
                    name: st.to_string(),
                }),
            types,
            directives,
            // TODO: Description missing on graphql_parser::schema::SchemaDefinition
            description: None,
        },
    }
}
