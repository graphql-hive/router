use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_graphql_analysis::{
                OnGraphqlAnalysisHookPayload, OnGraphqlAnalysisHookResult, Selection,
            },
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::RouterPlugin,
    },
    GraphQLError,
};

#[derive(Default)]
pub struct FieldNullingPlugin;

#[async_trait]
impl RouterPlugin for FieldNullingPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "field_nulling"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_graphql_analysis<'exec>(
        &'exec self,
        payload: &mut OnGraphqlAnalysisHookPayload<'exec>,
    ) -> OnGraphqlAnalysisHookResult {
        payload.filter_operation(|selection| match selection {
            Selection::Field(field)
                if field.parent_type_name == "User" && field.field_name == "email" =>
            {
                selection.reject(GraphQLError::from_message_and_code(
                    "Access to field 'email' is not allowed",
                    "FIELD_ACCESS_DENIED",
                ))
            }
            _ => selection.keep(),
        });

        OnGraphqlAnalysisHookResult::Proceed
    }
}
