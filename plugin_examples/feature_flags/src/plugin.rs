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
            on_supergraph_load::{
                OnSupergraphLoadStartHookPayload, OnSupergraphLoadStartHookResult,
            },
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
};

#[derive(Default)]
struct FeatureFlagSchemas {
    supergraph: Option<Arc<Document>>,
    variants: HashMap<String, Arc<Document>>,
}

#[derive(Default)]
pub struct FeatureFlagsPlugin {
    schemas: Mutex<FeatureFlagSchemas>,
}

#[async_trait]
impl RouterPlugin for FeatureFlagsPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "feature_flags"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    fn on_supergraph_reload<'exec>(
        &'exec self,
        payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        let mut schemas = self.schemas.lock().unwrap();
        schemas.supergraph = Some(Arc::new(payload.new_ast.clone()));
        schemas.variants.clear();
        payload.proceed()
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
        let mut feature_flags: Vec<String> = feature_flags_header
            .split(',')
            .map(|tag| tag.trim().to_string())
            .collect();

        // sort the feature flags so header order does not change the cache key
        feature_flags.sort();

        let cache_key = feature_flags.join(",");
        let mut schemas = self.schemas.lock().unwrap();
        let Some(supergraph) = schemas.supergraph.clone() else {
            return payload.proceed();
        };
        let document = schemas
            .variants
            .entry(cache_key)
            .or_insert_with(|| schema_for_features(&supergraph, &feature_flags))
            .clone();
        drop(schemas);

        payload.set_schema_document(document);
        payload.proceed()
    }
}

fn schema_for_features(document: &Arc<Document>, feature_flags: &[String]) -> Arc<Document> {
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

    Arc::new(Document { definitions })
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
