use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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

/// This example shows how a plugin can be the router's only source of a supergraph
/// (`supergraph.source: plugin` in `router.config.yaml`) and pick a feature-flag-stripped variant
/// of it per request, in `on_http_request`.
///
/// The base supergraph document is parsed once, in `on_plugin_init`. From then on, every request
/// builds (or reuses a cached) `Arc<Supergraph>` for its exact combination of
/// `x-feature-flags` header values, stripping any `@feature`-tagged types/fields that aren't
/// enabled - affecting parsing, validation, planning, execution *and* introspection alike.
pub struct FeatureFlagsPlugin {
    supergraph: Document,
    variants: Mutex<HashMap<String, Arc<Supergraph>>>,
}

#[async_trait]
impl RouterPlugin for FeatureFlagsPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "feature_flags"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let subgraphs_url = std::env::var("FEATURE_FLAGS_SUBGRAPHS_URL")
            .expect("FEATURE_FLAGS_SUBGRAPHS_URL must be set for tests");
        let document =
            safe_parse_schema(&SUPERGRAPH_SDL.replace("http://0.0.0.0:4200", &subgraphs_url))?;
        payload.initialize_plugin(Self {
            supergraph: document,
            variants: Mutex::new(HashMap::new()),
        })
    }

    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        let feature_flags_header = payload
            .router_http_request
            .headers()
            .get("x-feature-flags")
            .and_then(|header_value| header_value.to_str().ok())
            .unwrap_or_default();

        if feature_flags_header == "skip" {
            // skip setting the supergraph to test NO_SUPERGRAPH_AVAILABLE
            return payload.proceed();
        }

        let mut feature_flags: Vec<String> = feature_flags_header
            .split(',')
            .map(|tag| tag.trim().to_string())
            .collect();

        // sort the feature flags so header order does not change the cache key
        feature_flags.sort();
        let cache_key = feature_flags.join(",");

        let mut variants = self.variants.lock().unwrap();
        let existing = variants.get(&cache_key).cloned();
        match existing {
            // (cheap) already constructed, serve from cache
            Some(selected) => payload.set_supergraph(selected),
            // (expensive) not constructed, build and cache it
            None => {
                let document = schema_for_features(&self.supergraph, &feature_flags);
                match Supergraph::from_document(document, Default::default()) {
                    Ok(supergraph_data) => {
                        // build successful
                        let supergraph_data = Arc::new(supergraph_data);
                        variants.insert(cache_key, supergraph_data.clone());
                        payload.set_supergraph(supergraph_data);
                    }
                    Err(_) => {
                        // building the supergraph failed, you can optionally log, but
                        // intentionally not set the supergraph and let the router fail the
                        // request with an error coded NO_SUPERGRAPH_AVAILABLE
                    }
                }
            }
        }

        payload.proceed()
    }
}

fn schema_for_features(document: &Document, feature_flags: &[String]) -> Document {
    let mut removed_definitions = vec![];
    let mut removed_field_definitions: HashMap<String, Vec<String>> = HashMap::new();

    for definition in &document.definitions {
        if let Some(directives) = definition.directives() {
            if !should_keep_node(feature_flags, directives) {
                if let Some(name) = definition.name() {
                    removed_definitions.push(name.to_string());
                }
                continue;
            }
        }

        let fields = match definition.fields() {
            Some(TypeDefinitionFields::Fields(fields)) => fields
                .iter()
                .map(|field| (&field.name, &field.directives))
                .collect::<Vec<_>>(),
            Some(TypeDefinitionFields::InputValues(fields)) => fields
                .iter()
                .map(|field| (&field.name, &field.directives))
                .collect(),
            Some(TypeDefinitionFields::EnumValues(fields)) => fields
                .iter()
                .map(|field| (&field.name, &field.directives))
                .collect(),
            None => continue,
        };

        for (field_name, directives) in fields {
            if !should_keep_node(feature_flags, directives) {
                if let Some(definition_name) = definition.name() {
                    removed_field_definitions
                        .entry(definition_name.to_string())
                        .or_default()
                        .push(field_name.clone());
                }
            }
        }
    }

    if removed_definitions.is_empty() && removed_field_definitions.is_empty() {
        return document.clone();
    }

    let definitions = document
        .definitions
        .iter()
        .filter(|definition| {
            definition.name().map_or(true, |name| {
                !removed_definitions.iter().any(|removed| removed == name)
            })
        })
        .map(|definition| {
            let Some(name) = definition.name() else {
                return definition.clone();
            };
            let Some(removed_fields) = removed_field_definitions.get(name) else {
                return definition.clone();
            };

            match definition {
                Definition::TypeDefinition(TypeDefinition::Object(object)) => {
                    Definition::TypeDefinition(TypeDefinition::Object(ObjectType {
                        fields: object
                            .fields
                            .iter()
                            .filter(|field| !removed_fields.contains(&field.name))
                            .cloned()
                            .collect(),
                        ..object.clone()
                    }))
                }
                Definition::TypeDefinition(TypeDefinition::InputObject(input_object)) => {
                    Definition::TypeDefinition(TypeDefinition::InputObject(InputObjectType {
                        fields: input_object
                            .fields
                            .iter()
                            .filter(|field| !removed_fields.contains(&field.name))
                            .cloned()
                            .collect(),
                        ..input_object.clone()
                    }))
                }
                Definition::TypeDefinition(TypeDefinition::Enum(enum_definition)) => {
                    Definition::TypeDefinition(TypeDefinition::Enum(EnumType {
                        values: enum_definition
                            .values
                            .iter()
                            .filter(|value| !removed_fields.contains(&value.name))
                            .cloned()
                            .collect(),
                        ..enum_definition.clone()
                    }))
                }
                _ => definition.clone(),
            }
        })
        .collect();

    Document { definitions }
}

fn should_keep_node(
    feature_flags: &[String],
    directives: &[hive_router::graphql_tools::static_graphql::schema::Directive],
) -> bool {
    for directive in directives {
        if directive.name.as_str() == "feature" {
            for (argument_name, argument_val) in &directive.arguments {
                if argument_name == "name" {
                    if let Value::String(feature_tag) = argument_val {
                        if !feature_flags.contains(feature_tag) {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
}
