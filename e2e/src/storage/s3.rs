#[cfg(test)]
mod storage_s3_e2e_tests {
    use std::time::Duration;

    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{s3_mock::S3Mock, ClientResponseExt, TestRouter};

    #[ntex::test]
    async fn should_load_supergraph_from_storage() {
        let storage = S3Mock::start("test-bucket").await;
        let first_supergraph = include_str!("../../supergraph.graphql");
        let location = "my-dir/supergraph.graphql";
        storage.set(location, first_supergraph.as_bytes()).await;

        let config = format!(
            r#"
            storages:
              test:
                type: s3
                bucket: {}
                endpoint: {}
                allow_http: true
                credentials:
                  type: static
                  access_key_id: {}
                  secret_access_key: {}
            supergraph:
              source: storage
              storage_id: test
              location: {}
            "#,
            storage.bucket(),
            storage.url(),
            storage.access_key(),
            storage.secret_key(),
            location
        );

        let router = TestRouter::builder()
            .inline_config(config)
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
    }

    #[ntex::test]
    async fn should_poll_and_load_supergraph_from_storage() {
        let storage = S3Mock::start("test-bucket").await;
        let first_supergraph = include_str!("../../supergraph.graphql");
        let location = "my-dir/supergraph.graphql";
        storage.set(location, first_supergraph.as_bytes()).await;

        let config = format!(
            r#"
            storages:
              test:
                type: s3
                bucket: {}
                endpoint: {}
                allow_http: true
                credentials:
                  type: static
                  access_key_id: {}
                  secret_access_key: {}
            supergraph:
              source: storage
              storage_id: test
              location: {}
              poll_interval: 100ms
            "#,
            storage.bucket(),
            storage.url(),
            storage.access_key(),
            storage.secret_key(),
            location
        );

        let router = TestRouter::builder()
            .inline_config(config)
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        storage
            .set(
                location,
                "type Query { dummyNew: NewType } type NewType { id: ID! }".as_bytes(),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;
        let types_arr: Vec<String> = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i.as_object()
                    .unwrap()
                    .get(&"name")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            types_arr.contains(&"Query".to_string()),
            true,
            "Expected types to contain 'Query'"
        );
        assert_eq!(
            types_arr.contains(&"NewType".to_string()),
            true,
            "Expected types to contain 'NewType'"
        );
    }
}
