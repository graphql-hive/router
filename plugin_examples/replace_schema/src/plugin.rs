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
            on_supergraph_load::Supergraph,
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    query_planner::utils::parsing::safe_parse_schema,
};

const SUPERGRAPH_SDL: &str = include_str!("../supergraph.graphql");

/// This example shows how a plugin can override the router's default, configured supergraph
/// (`supergraph.source: file` in `router.config.yaml`) with a different one, per request, in
/// `on_http_request`.
///
/// Overriding a supergraph also affects introspection, because introspection is built from the
/// same schema snapshot that parsing/validation/planning use.
///
/// Here we build one extra `Arc<Supergraph>` (stripping `@feature`-tagged types/fields from
/// the *supergraph* document, not the public schema - the public/consumer schema is derived by
/// the router from whatever we hand it) and swap it in only when the `x-schema-variant: basic`
/// request header is present. Without that header (or with any other value), the router's own
/// default supergraph is used unchanged.
pub struct ReplaceSchemaPlugin {
    basic_variant: Arc<Supergraph>,
}

#[async_trait]
impl RouterPlugin for ReplaceSchemaPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "replace_schema"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let subgraphs_url = std::env::var("REPLACE_SCHEMA_SUBGRAPHS_URL")
            .expect("REPLACE_SCHEMA_SUBGRAPHS_URL must be set for tests");
        let document =
            safe_parse_schema(&SUPERGRAPH_SDL.replace("http://0.0.0.0:4200", &subgraphs_url))?;
        let document = strip_disabled_features(document, &["inStock", "shippingEstimate"]);
        let basic_variant = Arc::new(Supergraph::from_document(document, Default::default())?);
        payload.initialize_plugin(Self { basic_variant })
    }

    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        let variant = payload
            .router_http_request
            .headers()
            .get("x-schema-variant")
            .and_then(|value| value.to_str().ok());

        if variant == Some("basic") {
            payload.set_supergraph(self.basic_variant.clone());
        }

        payload.proceed()
    }
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
