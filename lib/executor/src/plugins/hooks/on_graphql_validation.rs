use std::sync::Arc;

use graphql_tools::{
    static_graphql::{query::Document as QueryDocument, schema::Document as SchemaDocument},
    validation::{rules::ValidationRule, utils::ValidationError, validate::ValidationPlan},
};
use hive_router_query_planner::consumer_schema::ConsumerSchema;
use ntex::http::Response;

use crate::{
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{CacheHint, EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLValidationStartHookPayload<'exec> {
    /// The incoming HTTP request to the router for which the GraphQL execution is happening.
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```
    ///  let my_header = payload.router_http_request.headers.get("my-header");
    ///  // do something with the header...
    ///  payload.proceed()
    /// ```
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    /// The GraphQL Schema that the document will be validated against.
    /// This is not the same with the supergraph. This is the public schema exposed by the router to the clients, which is generated from the supergraph and can be modified by the plugins.
    /// The plugins can replace the input schema to be used for validation
    /// and the new schema will be used in the validation process instead of the original one.
    ///
    /// [See an example to see when to override the schema](https://github.com/graphql-hive/router/blob/main/plugin_examples/feature_flags/src/plugin.rs)
    pub schema: Arc<ConsumerSchema>,
    /// Parsed GraphQL document from the query string in the GraphQL parameters.
    /// It contains the Abstract Syntax Tree (AST) representation of the GraphQL query, mutation, or subscription
    /// sent by the client in the request body.
    ///
    /// But the plugins can replace the input document AST to be used for validation
    /// and the new document will be used in the validation process instead of the original one.
    pub document: Arc<QueryDocument>,
    /// The set of rules to be used in the validation process.
    /// The plugins can modify the validation rules to be used in the validation process by adding new rules or
    /// removing existing ones.
    ///
    /// [See an example](https://github.com/graphql-hive/router/blob/main/plugin_examples/root_field_limit/src/lib.rs#:~:text=fn%20on_graphql_validation)
    pub validation_plan: Arc<ValidationPlan>,
    /// Overriding the validation errors to be used in the execution instead of the ones generated from the validation process.
    /// This is useful for plugins that want to generate custom validation errors in a custom way,
    /// or want to override the validation errors for testing or other purposes.
    ///
    /// [Learn more about overriding the default behavior](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#overriding-default-behavior)
    pub errors: Option<Arc<Vec<ValidationError>>>,
}

impl OnGraphQLValidationStartHookPayload<'_> {
    /// Override validation rules to be used in the validation process by adding a new rule.
    /// [See an example](https://github.com/graphql-hive/router/blob/main/plugin_examples/root_field_limit/src/lib.rs#:~:text=fn%20on_graphql_validation)
    pub fn with_validation_plan<TValidationPlan: Into<ValidationPlan>>(
        mut self,
        validation_plan: TValidationPlan,
    ) -> Self {
        self.validation_plan = Arc::new(validation_plan.into());
        self
    }
    /// Override the GraphQL Schema that the document will be validated against.
    /// [See an example to see when to override the schema](https://github.com/graphql-hive/router/blob/main/plugin_examples/feature_flags/src/plugin.rs)
    pub fn with_schema<TSchema: Into<Arc<SchemaDocument>>>(mut self, schema: TSchema) -> Self {
        let schema: Arc<SchemaDocument> = schema.into();
        let new_consumer_schema = ConsumerSchema::from(schema);
        self.schema = new_consumer_schema.into();
        self
    }
}

impl<'exec> StartHookPayload<OnGraphQLValidationEndHookPayload, Response>
    for OnGraphQLValidationStartHookPayload<'exec>
{
}

pub type OnGraphQLValidationStartHookResult<'exec> = StartHookResult<
    'exec,
    OnGraphQLValidationStartHookPayload<'exec>,
    OnGraphQLValidationEndHookPayload,
    Response,
>;

impl<'exec> OnGraphQLValidationStartHookPayload<'exec> {
    /// Adds a new validation rule to the existing set of rules to be used in the validation process.
    /// [See an example](https://github.com/graphql-hive/router/blob/main/plugin_examples/root_field_limit/src/lib.rs#:~:text=fn%20on_graphql_validation)
    pub fn with_validation_rule<TValidationRule: ValidationRule + 'static>(
        mut self,
        rule: TValidationRule,
    ) -> Self {
        let mut new_plan = self.validation_plan.as_ref().clone();
        new_plan.add_rule(Box::new(rule));
        self.validation_plan = Arc::new(new_plan);
        self
    }

    /// Filters the existing validation rules to be used in the validation process by removing the rules that don't satisfy the given predicate function.
    pub fn filter_validation_rules<F>(mut self, mut f: F) -> Self
    where
        F: FnMut(&Box<dyn ValidationRule>) -> bool,
    {
        let mut new_plan = self.validation_plan.as_ref().clone();
        new_plan.rules.retain(|rule| f(rule));
        self.validation_plan = Arc::new(new_plan);
        self
    }
}

pub struct OnGraphQLValidationEndHookPayload {
    pub errors: Arc<Vec<ValidationError>>,
    pub cache_hint: CacheHint,
}

impl EndHookPayload<Response> for OnGraphQLValidationEndHookPayload {}

pub type OnGraphQLValidationHookEndResult =
    EndHookResult<OnGraphQLValidationEndHookPayload, Response>;
