use hive_router_plan_executor::{
    hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    plugin_trait::{RouterPlugin, StartHookPayload},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// A usage reporting plugin that sends everything on shutdown. Don't ask why? Testing...

#[derive(Serialize)]
struct UsageReport {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_name: Option<String>,
}

struct UsageReportingPlugin {
    endpoint: String,
    reports: Mutex<Vec<UsageReport>>,
}

#[derive(Deserialize)]
pub struct UsageReportingPluginConfig {
    enabled: bool,
    endpoint: String,
}

#[async_trait::async_trait]
impl RouterPlugin for UsageReportingPlugin {
    type Config = UsageReportingPluginConfig;
    fn plugin_name() -> &'static str {
        "usage_reporting"
    }
    fn from_config(config: UsageReportingPluginConfig) -> Option<Self> {
        if config.enabled {
            Some(UsageReportingPlugin {
                endpoint: config.endpoint,
                reports: Default::default(),
            })
        } else {
            None
        }
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
        payload.cont()
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
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::PluginRegistry;
    use ntex::web::test;
    use serde_json::json;
    #[ntex::test]
    async fn test_usage_reporting_plugin() -> Result<(), Box<dyn std::error::Error>> {
        let query = "query Test {me{name}}";
        let operation_name = "Test";
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
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

            let test_app = init_router_from_config_inline(
                &format!(
                    r#"
                plugins:
                    usage_reporting:
                        enabled: true
                        endpoint: "http://{}/usage_report"
                "#,
                    host
                ),
                Some(PluginRegistry::new().register::<super::UsageReportingPlugin>()),
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
