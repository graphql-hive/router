use async_trait::async_trait;

use crate::{ExecutionRequest, ExecutionResult};

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult;
}

pub type SubgraphExecutorType<'a> = dyn SubgraphExecutor + Send + Sync + 'a;
