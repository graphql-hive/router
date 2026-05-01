use mockito::{Mock, ServerGuard};
use sonic_rs::JsonValueTrait;

pub struct TestCoprocessor {
    pub server: ServerGuard,
}

impl TestCoprocessor {
    pub async fn new() -> Self {
        Self {
            server: mockito::Server::new_async().await,
        }
    }

    pub fn host_with_port(&self) -> String {
        self.server.host_with_port()
    }

    /// Creates a mock that matches a specific coprocessor stage.
    pub fn mock_stage(&mut self, stage_name: impl Into<String>) -> Mock {
        let stage_name = stage_name.into();
        self.server
            .mock("POST", "/coprocessor")
            .match_request(move |request| {
                let Ok(body) = request.body() else {
                    return false;
                };
                let Ok(payload) = sonic_rs::from_slice::<sonic_rs::Value>(body) else {
                    return false;
                };
                payload
                    .get("stage")
                    .and_then(|value| value.as_str())
                    .is_some_and(|stage| stage == stage_name)
            })
    }

    /// Creates a mock that matches a specific coprocessor stage and an additional predicate on the parsed JSON body.
    pub fn mock_stage_with_matcher<F>(
        &mut self,
        stage_name: impl Into<String>,
        predicate: F,
    ) -> Mock
    where
        F: Fn(&sonic_rs::Value) -> bool + Send + Sync + 'static,
    {
        let stage_name = stage_name.into();
        self.server
            .mock("POST", "/coprocessor")
            .match_request(move |request| {
                let Ok(body) = request.body() else {
                    return false;
                };
                let Ok(payload) = sonic_rs::from_slice::<sonic_rs::Value>(body) else {
                    return false;
                };
                if payload.get("stage").and_then(|value| value.as_str())
                    != Some(stage_name.as_str())
                {
                    return false;
                }
                predicate(&payload)
            })
    }
}
