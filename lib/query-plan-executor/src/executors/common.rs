use async_trait::async_trait;

use crate::{ExecutionRequest, ExecutionResult};

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult;
}