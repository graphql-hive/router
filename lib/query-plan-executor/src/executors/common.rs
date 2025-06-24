use std::sync::Arc;

use async_trait::async_trait;

use crate::{execution_result::ExecutionResult, ExecutionRequest};

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult;
    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType = dyn crate::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;
