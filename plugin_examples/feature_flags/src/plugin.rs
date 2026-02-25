use std::{collections::HashMap, sync::Arc};

use hive_router::{
    async_trait,
    graphql_tools::{
        ast::TypeDefinitionFields,
        parser::schema::Definition,
        static_graphql::{query::Value, schema::TypeDefinition},
    },
    plugins::{
        hooks::{
            on_graphql_validation::{
                OnGraphQLValidationStartHookPayload, OnGraphQLValidationStartHookResult,
            },
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    query_planner::state::supergraph_state::SchemaDocument,
    DashMap,
};

#[derive(Default)]
pub struct FeatureFlagsPlugin {
    schema_with_flags_cache: DashMap<String, Arc<SchemaDocument>>,
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
    async fn on_graphql_validation<'exec>(
        &'exec self,
        payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        let feature_flags_header = payload
            .router_http_request
            .headers
            .get("x-feature-flags")
            .and_then(|header_value| header_value.to_str().ok())
            .unwrap_or_default();
        let mut feature_flags: Vec<String> = feature_flags_header
            .split(',')
            .map(|tag| tag.trim().to_string())
            .collect();

        // Let's sort the feature flags to ensure that the cache key is consistent regardless of the order of flags in the header
        feature_flags.sort();

        let cache_key = feature_flags.join(",");

        let cached_schema = self
            .schema_with_flags_cache
            .entry(cache_key)
            .or_insert_with(|| {
                let mut removed_definitions = vec![];
                let mut removed_field_definitions = HashMap::new();

                for definition in &payload.schema.document.definitions {
                    if let Some(directives) = definition.directives() {
                        if !should_keep_node(&feature_flags, directives) {
                            removed_definitions.push(definition);
                            continue;
                        }
                    }
                        match definition.fields() {
                            Some(TypeDefinitionFields::Fields(fields)) => {
                                for field_def in fields {
                                    if !should_keep_node(&feature_flags, &field_def.directives) {
                                        if let Some(def_name) = definition.name() {
                                            removed_field_definitions
                                                .entry(def_name)
                                                .or_insert_with(Vec::new)
                                                .push(field_def.name.as_str())
                                        }
                                    }
                                }
                            }
                            Some(TypeDefinitionFields::InputValues(input_values)) => {
                                for input_value_def in input_values {
                                    if !should_keep_node(
                                        &feature_flags,
                                        &input_value_def.directives,
                                    ) {
                                        if let Some(def_name) = definition.name() {
                                            removed_field_definitions
                                                .entry(def_name)
                                                .or_insert_with(Vec::new)
                                                .push(input_value_def.name.as_str());
                                        }
                                    }
                                }
                            }
                            Some(TypeDefinitionFields::EnumValues(enum_values)) => {
                                for enum_value_def in enum_values {
                                    if !should_keep_node(&feature_flags, &enum_value_def.directives)
                                    {
                                        if let Some(def_name) = definition.name() {
                                            removed_field_definitions
                                                .entry(def_name)
                                                .or_insert_with(Vec::new)
                                                .push(enum_value_def.name.as_str());
                                        }
                                    }
                                }
                            }
                            None => {}
                        }
                }

                if removed_definitions.is_empty() && removed_field_definitions.is_empty() {
                    Arc::clone(&payload.schema.document)
                } else {
                    let new_definitions = payload
                        .schema
                        .document
                        .definitions
                        .iter()
                        .filter(|def| !removed_definitions.contains(def))
                        .map(|def| {
                            let Some(def_name) = def.name() else {
                                return def.clone();
                            };
                            let Some(removed_fields_for_def) = removed_field_definitions.get(def_name) else {
                                return def.clone();
                            };
                            match def {
                                Definition::TypeDefinition(type_def) => match type_def {
                                    TypeDefinition::Object(obj) => {
                                        let new_fields = obj
                                            .fields
                                            .iter()
                                            .filter(|field| {
                                                !removed_fields_for_def.contains(&field.name.as_str())
                                            })
                                            .cloned()
                                            .collect();
                                        let new_obj = hive_router::graphql_tools::static_graphql::schema::ObjectType {
                                            fields: new_fields,
                                            ..obj.clone()
                                        };
                                        Definition::TypeDefinition(TypeDefinition::Object(new_obj))
                                    }
                                    TypeDefinition::InputObject(input_obj) => {
                                        let new_input_values = input_obj
                                            .fields
                                            .iter()
                                            .filter(|input_value| {
                                                !removed_fields_for_def.contains(&input_value.name.as_str())
                                            })
                                            .cloned()
                                            .collect();
                                        let new_input_obj = hive_router::graphql_tools::static_graphql::schema::InputObjectType {
                                            fields: new_input_values,
                                            ..input_obj.clone()
                                        };
                                        Definition::TypeDefinition(TypeDefinition::InputObject(new_input_obj))
                                    }
                                    TypeDefinition::Enum(enum_def) => {
                                        let new_enum_values = enum_def
                                            .values
                                            .iter()
                                            .filter(|enum_value| {
                                                !removed_fields_for_def.contains(&enum_value.name.as_str())
                                            })
                                            .cloned()
                                            .collect();
                                        let new_enum_def = hive_router::graphql_tools::static_graphql::schema::EnumType {
                                            values: new_enum_values,
                                            ..enum_def.clone()
                                        };
                                        Definition::TypeDefinition(TypeDefinition::Enum(new_enum_def))
                                    }
                                    _ => def.clone(),
                                }
                                _ => def.clone(),
                            }
                        })
                        .collect();
                    Arc::new(SchemaDocument {
                        definitions: new_definitions,
                    })
                }
            })
            .value()
            .clone();

        payload.with_schema(cached_schema).proceed()
    }
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
