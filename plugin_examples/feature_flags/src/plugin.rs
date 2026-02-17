use std::sync::Arc;

use hive_router::{
    async_trait,
    graphql_tools::{
        ast::SchemaVisitor,
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
                let visitor = FeatureFlagsVisitor { feature_flags };

                visitor
                    .visit_schema_document(SchemaDocument::clone(&payload.schema.document), &mut ())
                    .unwrap()
                    .into()
            })
            .value()
            .clone();

        payload.with_schema(cached_schema).proceed()
    }
}

pub struct FeatureFlagsVisitor {
    feature_flags: Vec<String>,
}

impl SchemaVisitor<()> for FeatureFlagsVisitor {
    fn enter_type_definition(
        &self,
        type_definition: TypeDefinition,
        _visitor_context: &mut (),
    ) -> Option<TypeDefinition> {
        for directive in type_definition.directives() {
            if directive.name.as_str() == "feature" {
                for (argument_name, argument_val) in &directive.arguments {
                    if argument_name == "name" {
                        if let Value::String(feature_tag) = argument_val {
                            if self.feature_flags.contains(feature_tag) {
                                return Some(type_definition);
                            } else {
                                return None;
                            }
                        }
                    }
                }
            }
        }
        Some(type_definition)
    }
    fn enter_object_type_field(
        &self,
        node: hive_router::graphql_tools::static_graphql::schema::Field,
        _type_: &hive_router::graphql_tools::static_graphql::schema::ObjectType,
        _visitor_context: &mut (),
    ) -> Option<hive_router::graphql_tools::static_graphql::schema::Field> {
        for directive in &node.directives {
            if directive.name.as_str() == "feature" {
                for (argument_name, argument_val) in &directive.arguments {
                    if argument_name == "name" {
                        if let Value::String(feature_tag) = argument_val {
                            if self.feature_flags.contains(feature_tag) {
                                return Some(node);
                            } else {
                                return None;
                            }
                        }
                    }
                }
            }
        }
        Some(node)
    }
}
