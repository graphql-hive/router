use std::{collections::HashMap, sync::Arc};

use hive_router::{
    async_trait,
    graphql_tools::{
        ast::TypeDefinitionFields,
        parser::schema::Definition,
        static_graphql::{
            query::Value,
            schema::{Document, EnumType, InputObjectType, ObjectType, TypeDefinition},
        },
    },
    plugins::{
        hooks::{
            on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    query_planner::utils::parsing::safe_parse_schema,
    HiveRouterConfig, SchemaState, SupergraphManagerError, TelemetryContext,
};

const SUPERGRAPH_SDL: &str = include_str!("../supergraph.graphql");

/// This example shows how a plugin can pick a whole `SchemaState` (not just a schema document)
/// per request, in `on_http_request`, and have it hold for the entire pipeline: parsing,
/// validation, normalization, planning, execution *and* introspection.
///
/// Unlike stripping the schema at the validation stage (see the `feature_flags` example),
/// overriding the `SchemaState` also affects introspection, because `IntrospectionContext` is
/// built from the same `SupergraphData` that parsing/validation/planning use.
///
/// Here we pre-build one `SchemaState` per feature bundle from the supergraph SDL (stripping
/// `@feature`-tagged types/fields from the *supergraph* document, not the public schema - the
/// public/consumer schema is derived by the router from whatever we hand it) and pick between
/// them using the `x-schema-variant` request header.
pub struct ReplaceSchemaPlugin {
    variants: HashMap<&'static str, Arc<SchemaState>>,
}

#[async_trait]
impl RouterPlugin for ReplaceSchemaPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "replace_schema"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let router_config = Arc::new(HiveRouterConfig::default());
        let telemetry_context = Arc::new(TelemetryContext::from_propagation_config(
            &Default::default(),
        ));

        let mut variants = HashMap::new();
        variants.insert(
            "full",
            Arc::new(build_schema_state(
                &[],
                router_config.clone(),
                telemetry_context.clone(),
            )?),
        );
        variants.insert(
            "basic",
            Arc::new(build_schema_state(
                &["inStock", "shippingEstimate"],
                router_config,
                telemetry_context,
            )?),
        );

        payload.initialize_plugin(Self { variants })
    }

    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        let variant = payload
            .router_http_request
            .headers()
            .get("x-schema-variant")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("full");

        if let Some(schema_state) = self.variants.get(variant) {
            payload.set_schema_state(schema_state.clone());
        }

        payload.proceed()
    }
}

/// Builds a `SchemaState` from the supergraph SDL, with the given `@feature`-tagged types/fields
/// stripped from the supergraph document (so they disappear from planning, validation *and*
/// introspection alike). An empty `disabled_features` keeps the schema as-is.
fn build_schema_state(
    disabled_features: &[&str],
    router_config: Arc<HiveRouterConfig>,
    telemetry_context: Arc<TelemetryContext>,
) -> Result<SchemaState, SupergraphManagerError> {
    let document = safe_parse_schema(SUPERGRAPH_SDL)?;
    let document = strip_disabled_features(document, disabled_features);
    SchemaState::from_supergraph_document(document, router_config, telemetry_context)
}

fn strip_disabled_features(document: Document, disabled_features: &[&str]) -> Document {
    let mut removed_definitions: Vec<String> = vec![];
    let mut removed_fields: HashMap<String, Vec<String>> = HashMap::new();

    for definition in &document.definitions {
        let Some(def_name) = definition.name() else {
            continue;
        };
        if let Some(directives) = definition.directives() {
            if has_disabled_feature(disabled_features, directives) {
                removed_definitions.push(def_name.to_string());
                continue;
            }
        }
        if let Some(TypeDefinitionFields::Fields(fields)) = definition.fields() {
            for field in fields {
                if has_disabled_feature(disabled_features, &field.directives) {
                    removed_fields
                        .entry(def_name.to_string())
                        .or_default()
                        .push(field.name.clone());
                }
            }
        }
    }

    let definitions = document
        .definitions
        .into_iter()
        .filter(|def| {
            def.name()
                .map_or(true, |name| !removed_definitions.iter().any(|d| d == name))
        })
        .map(|def| {
            let Some(def_name) = def.name().map(str::to_string) else {
                return def;
            };
            let Some(fields_to_remove) = removed_fields.get(&def_name) else {
                return def;
            };
            match def {
                Definition::TypeDefinition(TypeDefinition::Object(obj)) => {
                    Definition::TypeDefinition(TypeDefinition::Object(ObjectType {
                        fields: obj
                            .fields
                            .into_iter()
                            .filter(|field| !fields_to_remove.contains(&field.name))
                            .collect(),
                        ..obj
                    }))
                }
                Definition::TypeDefinition(TypeDefinition::InputObject(input_obj)) => {
                    Definition::TypeDefinition(TypeDefinition::InputObject(InputObjectType {
                        fields: input_obj
                            .fields
                            .into_iter()
                            .filter(|field| !fields_to_remove.contains(&field.name))
                            .collect(),
                        ..input_obj
                    }))
                }
                Definition::TypeDefinition(TypeDefinition::Enum(enum_def)) => {
                    Definition::TypeDefinition(TypeDefinition::Enum(EnumType {
                        values: enum_def
                            .values
                            .into_iter()
                            .filter(|value| !fields_to_remove.contains(&value.name))
                            .collect(),
                        ..enum_def
                    }))
                }
                other => other,
            }
        })
        .collect();

    Document { definitions }
}

/// Whether this node is tagged `@feature(name: ...)` for one of the disabled features.
fn has_disabled_feature(
    disabled_features: &[&str],
    directives: &[hive_router::graphql_tools::static_graphql::schema::Directive],
) -> bool {
    for directive in directives {
        if directive.name.as_str() != "feature" {
            continue;
        }
        for (argument_name, argument_value) in &directive.arguments {
            if argument_name == "name" {
                if let Value::String(feature_name) = argument_value {
                    if disabled_features.contains(&feature_name.as_str()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}
