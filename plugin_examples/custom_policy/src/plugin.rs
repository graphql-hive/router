use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_graphql_analysis::{OnGraphqlAnalysisHookPayload, OnGraphqlAnalysisHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::RouterPlugin,
    },
    tracing,
};

/// The header this example reads its authorization decision from. A real
/// plugin would look this up in a database, a permissions service, or decode
/// it from a JWT claim instead of trusting a raw header.
const ROLE_HEADER: &str = "x-user-role";
const ADMIN_ROLE: &str = "admin";

/// Demonstrates deciding `@policy` policies from a plugin instead of a
/// coprocessor. Both share the exact same mechanism: the router publishes
/// every policy the operation depends on to the `graphql.analysis` stage's
/// request context, and whoever answers it - a coprocessor over HTTP, or a
/// plugin like this one, in-process - decides which of them are granted.
/// Anything left undecided is denied.
pub struct CustomPolicyPlugin;

#[async_trait]
impl RouterPlugin for CustomPolicyPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "custom_policy"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin(Self)
    }

    async fn on_graphql_analysis<'exec>(
        &'exec self,
        payload: &mut OnGraphqlAnalysisHookPayload<'exec>,
    ) -> OnGraphqlAnalysisHookResult {
        let required_policies: Vec<String> = match payload.request_context.read() {
            Ok(read) => read
                .authorization()
                .required_policies()
                .map(|policies| policies.keys().cloned().collect())
                .unwrap_or_default(),
            Err(err) => {
                tracing::error!("custom_policy: failed to read request context: {err}");
                Vec::new()
            }
        };

        // Nothing in this operation is policy-gated - skip touching the
        // context at all.
        if required_policies.is_empty() {
            return OnGraphqlAnalysisHookResult::Proceed;
        }

        let is_admin = payload
            .router_http_request
            .headers
            .get(ROLE_HEADER)
            .and_then(|value| value.to_str().ok())
            == Some(ADMIN_ROLE);

        match payload.request_context.write() {
            Ok(mut write) => {
                let mut authorization = write.authorization();
                for policy in required_policies {
                    // A real implementation would decide each policy on its
                    // own merits (e.g. does this user's role grant *this*
                    // specific policy). This example keeps it simple: the
                    // `admin` role grants every policy the operation
                    // requires, everyone else gets none of them.
                    authorization.set_policy_decision(policy, is_admin);
                }
            }
            Err(err) => tracing::error!("custom_policy: failed to write request context: {err}"),
        }

        OnGraphqlAnalysisHookResult::Proceed
    }
}
