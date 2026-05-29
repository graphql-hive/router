#[cfg(test)]
mod storage_s3_e2e_tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{s3_mock::S3Mock, ClientResponseExt, TestRouter};

    /// Serializes the tests that mutate process-global `AWS_*` environment
    /// variables. The test harness runs tests in parallel within a single
    /// process, so without this they would clobber one another's environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that sets environment variables for the duration of a test and
    /// restores their previous values on drop. Holds [`ENV_LOCK`] while alive so
    /// no other env-mutating test runs concurrently.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, &str)]) -> Self {
            // Recover from a poisoned lock so a single panicking test does not
            // wedge the rest of the suite.
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let saved = vars
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    std::env::set_var(key, value);
                    (key.to_string(), previous)
                })
                .collect();
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in &self.saved {
                match previous {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    /// With no `credentials` block in the config, the backend must pick up
    /// credentials from the standard `AWS_*` environment variables. This is the
    /// same `from_env` mechanism that makes EKS IRSA work out of the box (the
    /// pod identity webhook injects `AWS_WEB_IDENTITY_TOKEN_FILE`/`AWS_ROLE_ARN`).
    #[ntex::test]
    async fn should_load_supergraph_using_credentials_from_env() {
        let storage = S3Mock::start("test-bucket").await;
        let supergraph = include_str!("../../supergraph.graphql");
        let location = "my-dir/supergraph.graphql";
        storage.set(location, supergraph.as_bytes()).await;

        // Credentials are supplied through the environment; only non-credential
        // settings remain in the config.
        let _env = EnvGuard::set(&[
            ("AWS_ACCESS_KEY_ID", storage.access_key()),
            ("AWS_SECRET_ACCESS_KEY", storage.secret_key()),
        ]);

        let config = format!(
            r#"
            storages:
              test:
                type: s3
                bucket: {}
                endpoint: {}
                allow_http: true
            supergraph:
              source: storage
              storage_id: test
              location: {}
            "#,
            storage.bucket(),
            storage.url(),
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

        assert!(
            res.status().is_success(),
            "Expected 200 OK when credentials are supplied via AWS_* env vars"
        );
    }

    /// Explicit credentials in the config are used verbatim and the `AWS_*`
    /// credential environment variables are ignored entirely. This guards against
    /// env credentials mixing into the configured ones — in particular a stray
    /// `AWS_SESSION_TOKEN` must not attach to configured static keys (which would
    /// break signing), and bogus env access keys must not be used at all.
    #[ntex::test]
    async fn should_prefer_config_credentials_over_env() {
        let storage = S3Mock::start("test-bucket").await;
        let supergraph = include_str!("../../supergraph.graphql");
        let location = "my-dir/supergraph.graphql";
        storage.set(location, supergraph.as_bytes()).await;

        // Wrong credentials in the environment, including a session token that
        // must not leak onto the configured token-less static credentials.
        let _env = EnvGuard::set(&[
            ("AWS_ACCESS_KEY_ID", "AKIAWRONGWRONGWRONG0"),
            ("AWS_SECRET_ACCESS_KEY", "wrong-secret-should-be-ignored"),
            ("AWS_SESSION_TOKEN", "bogus-session-token-should-be-ignored"),
        ]);

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

        assert!(
            res.status().is_success(),
            "Expected config credentials to override the bogus AWS_* env vars"
        );
    }

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
