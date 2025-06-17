use async_graphql::{Request, Response};
use async_trait::async_trait;

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute(&self, execution_request: Request) -> Response;
}

pub type SubgraphExecutorType<'a> = dyn SubgraphExecutor + Send + Sync + 'a;

#[async_trait]
impl<Executor> SubgraphExecutor for Executor
where
    Executor: async_graphql::Executor,
{
    async fn execute(&self, execution_request: Request) -> Response {
        let response: async_graphql::Response =
            self.execute(execution_request).await;
        response
    }
}
