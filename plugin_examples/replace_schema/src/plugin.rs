use std::{collections::HashMap, sync::Arc};

use hive_router::{
    async_trait,
    graphql_tools::{
        ast::TypeDefinitionFields,
        parser::schema::{Definition, ParseError},
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
};

const SUPERGRAPH_SDL: &str = include_str!("../supergraph.graphql");

/// This example shows how a plugin can pick a whole schema document (not just a validation-time
/// schema) per request, in `on_http_request`, and have it hold for the entire pipeline: parsing,
/// validation, normalization, planning, execution *and* introspection.
///
/// Unlike stripping the schema at the validation stage (see the `feature_flags` example),
/// overriding the schema document also affects introspection, because introspection is built
/// from the same resolved schema state that parsing/validation/planning use.
///
/// Here we pre-parse one supergraph document per feature bundle (stripping `@feature`-tagged
/// types/fields from the *supergraph* document, not the public schema - the public/consumer
/// schema is derived by the router from whatever we hand it) and pick between them using the
/// `x-schema-variant` request header.
///
/// The plugin only owns stable `Arc<Document>`s. The router resolves each document to its own
/// internally-owned `SchemaState` (building it once on the first request that selects it, then
/// reusing it for later requests with the same `Arc<Document>`).
pub struct ReplaceSchemaPlugin {
    variants: HashMap<&'static str, Arc<Document>>,
}

#[async_trait]
impl RouterPlugin for ReplaceSchemaPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "replace_schema"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        // Parse each schema variant once here and keep the `Arc<Document>` around for the
        // lifetime of the plugin. `on_http_request` must hand the router the *same* `Arc` every
        // time a variant is selected - constructing a fresh document per request would defeat
        // the router's schema-state cache and force an expensive rebuild on every request.
        let mut variants = HashMap::new();
        variants.insert("full", Arc::new(build_document(&[])?));
        variants.insert(
            "basic",
            Arc::new(build_document(&["inStock", "shippingEstimate"])?),
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

        if let Some(document) = self.variants.get(variant) {
            payload.set_schema_document(document.clone());
        }

        payload.proceed()
    }
}

/// Builds a supergraph document with the given `@feature`-tagged types/fields stripped (so they
/// disappear from planning, validation *and* introspection alike). An empty `disabled_features`
/// keeps the schema as-is.
fn build_document(disabled_features: &[&str]) -> Result<Document, ParseError> {
    let document = safe_parse_schema(SUPERGRAPH_SDL)?;
    Ok(strip_disabled_features(document, disabled_features))
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
