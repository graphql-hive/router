mod s3;

#[cfg(test)]
mod storage_e2e_tests {
    use crate::testkit::TestRouter;

    /// The goal of this test is to assert that the Router panics and throws at the config level when it starts
    /// This is needed in order to ensure config is checked statically and validated at startup
    #[ntex::test]
    #[should_panic(
        expected = "failed to configure hive router from config: SupergraphManagerError(LoadSupergraphError(StorageIdNotFound(\"missing\")))"
    )]
    async fn should_throw_when_storage_is_missing_in_supergraph() {
        TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: storage
                    storage_id: missing
                    location: test
                "#,
            ))
            .build()
            .start()
            .await;
    }

    #[ntex::test]
    #[should_panic(
        expected = "failed to configure hive router from config: SharedStateError(PersistedDocuments(StorageNotFound(\"missing\")))"
    )]
    async fn should_throw_when_storage_is_missing_in_persisted_docs() {
        TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: storage
                    storage_id: missing
                    location: test.json
                "#,
            ))
            .build()
            .start()
            .await;
    }
}
