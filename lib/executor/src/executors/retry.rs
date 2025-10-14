use async_trait::async_trait;
use hive_router_config::traffic_shaping::TrafficShapingExecutorConfig;
use tokio_retry2::{strategy::ExponentialBackoff, Retry, RetryError};

use crate::executors::common::{
    HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor, SubgraphExecutorBoxedArc,
};

pub struct RetryExecutor {
    pub executor: SubgraphExecutorBoxedArc,
    pub strategy: std::iter::Take<ExponentialBackoff>,
}

impl RetryExecutor {
    pub fn new(executor: SubgraphExecutorBoxedArc, config: &TrafficShapingExecutorConfig) -> Self {
        let retry_delay_as_millis = config.retry_delay.as_millis();
        let strategy = ExponentialBackoff::from_millis(retry_delay_as_millis as u64)
            .factor(config.retry_factor)
            .max_delay(config.retry_delay)
            .take(config.max_retries + 1); // to account for the initial attempt
        Self { executor, strategy }
    }
}

#[async_trait]
impl SubgraphExecutor for RetryExecutor {
    async fn execute<'a>(
        &self,
        execution_request: &'a HttpExecutionRequest<'a>,
    ) -> HttpExecutionResponse {
        let action = async move || {
            let result = self.executor.execute(execution_request).await;
            if result.status.is_success() {
                Ok(result)
            } else {
                let retry_after_header = result
                    .headers
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                let retry_after = retry_after_header.map(std::time::Duration::from_secs);
                Err(RetryError::Transient {
                    err: result,
                    retry_after,
                })
            }
        };
        let result = Retry::spawn(self.strategy.clone(), action).await;

        match result {
            Ok(response) => response,
            Err(response) => response,
        }
    }
}
