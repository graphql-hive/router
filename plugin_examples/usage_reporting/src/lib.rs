use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    tokio::sync::Mutex,
    tracing,
};
use serde::{Deserialize, Serialize};

/// A usage reporting plugin that sends everything on shutdown. Don't ask why? Testing...

#[derive(Serialize)]
struct UsageReport {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_name: Option<String>,
}

pub struct UsageReportingPlugin {
    endpoint: String,
    reports: Mutex<Vec<UsageReport>>,
}

#[derive(Deserialize)]
pub struct UsageReportingPluginConfig {
    endpoint: String,
}

#[async_trait]
impl RouterPlugin for UsageReportingPlugin {
    type Config = UsageReportingPluginConfig;
    fn plugin_name() -> &'static str {
        "usage_reporting"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin(Self {
            endpoint: payload.config()?.endpoint,
            reports: Default::default(),
        })
    }
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        self.reports.lock().await.push(UsageReport {
            query: payload.operation_for_plan.to_string(),
            operation_name: payload.operation_for_plan.name.clone(),
        });
        tracing::trace!(
            "Pushed usage report for operation: {:?}",
            payload.operation_for_plan.name
        );
        payload.proceed()
    }
    async fn on_shutdown<'exec>(&'exec self) {
        println!("Disposing UsageReportingPlugin and sending usage report");
        // Here you would gather and send the usage report
        let reports = self.reports.lock().await;
        match reqwest::Client::new()
            .post(&self.endpoint)
            .json(reports.as_slice())
            .send()
            .await
        {
            Ok(response) => {
                tracing::trace!("Usage report sent successfully: {:?}", response);
            }
            Err(e) => {
                tracing::trace!("Failed to send usage report: {:?}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use e2e::mockito::{self, ServerOpts};
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        SubgraphsServer,
    };
    use hive_router::ntex::web::test;
    use hive_router::{ntex, BoxError, PluginRegistry};
    use serde_json::json;
    #[ntex::test]
    async fn test_usage_reporting_plugin() -> Result<(), BoxError> {
        let query = "query Test {me{name}}";
        let operation_name = "Test";
        let mut server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 9876,
            ..Default::default()
        })
        .await;
        let usage_mock = server
            .mock("POST", "/usage_report")
            .with_status(200)
            .expect(1)
            .match_body(mockito::Matcher::Json(json!([
                {
                    "query": query.to_string(),
                    "operation_name": operation_name.to_string(),
                }
            ])))
            .create_async()
            .await;
        {
            let _subgraphs_server = SubgraphsServer::start().await;

            let test_app = init_router_from_config_file_with_plugins(
                "../plugin_examples/usage_reporting/router.config.yaml",
                PluginRegistry::new().register::<super::UsageReportingPlugin>(),
            )
            .await?;

            wait_for_readiness(&test_app.app).await;

            let resp = test::call_service(
                &test_app.app,
                init_graphql_request(query, None).to_request(),
            )
            .await;
            assert!(resp.status().is_success(), "Expected 200 OK");

            test_app.shutdown().await;
        }

        usage_mock.assert_async().await;

        Ok(())
    }
}
