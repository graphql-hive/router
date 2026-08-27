#[cfg(test)]
mod tests {
    use std::time::Duration;

    use e2e::testkit::{some_header_map, Started, TestRouter, TestSubgraphs};
    use hive_router::ntex;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;

    const SECRET: &str = "test-secret";

    #[derive(Serialize)]
    struct Claims {
        sub: String,
        exp: usize,
    }

    fn jwt_cookie_for(sub: &str) -> String {
        let claims = Claims {
            sub: sub.to_string(),
            exp: unix_now_plus_hour(),
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
        format!("session={token}")
    }

    fn unix_now_plus_hour() -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        (now + 3600) as usize
    }

    async fn start_router(subgraphs: &TestSubgraphs<Started>) -> TestRouter<Started> {
        TestRouter::builder()
            .with_subgraphs(subgraphs)
            .inline_config(include_str!("../router.config.yaml").replace("${JWT_SECRET}", SECRET))
            .register_plugin::<crate::plugin::DedupePartitionPlugin>()
            .build()
            .start()
            .await
    }

    const QUERY: &str = r#"
        {
            topProducts {
                name
                price
            }
        }
    "#;

    #[ntex::test]
    async fn should_share_partition_for_the_same_jwt_subject() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
            .build()
            .start()
            .await;
        let router = start_router(&subgraphs).await;

        let cookie = jwt_cookie_for("alice");

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(QUERY, None, some_header_map! { "cookie" => &cookie },),
            router.send_graphql_request(QUERY, None, some_header_map! { "cookie" => &cookie },)
        );

        assert!(response_a.status().is_success());
        assert!(response_b.status().is_success());

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert_eq!(
            products_requests, 1,
            "expected requests with the same JWT subject to share a single dedupe partition"
        );
    }

    #[ntex::test]
    async fn should_not_share_partition_for_different_jwt_subjects() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
            .build()
            .start()
            .await;
        let router = start_router(&subgraphs).await;

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(
                QUERY,
                None,
                some_header_map! { "cookie" => &jwt_cookie_for("alice") },
            ),
            router.send_graphql_request(
                QUERY,
                None,
                some_header_map! { "cookie" => &jwt_cookie_for("bob") },
            )
        );

        assert!(response_a.status().is_success());
        assert!(response_b.status().is_success());

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert_eq!(
            products_requests, 2,
            "expected requests with different JWT subjects to fall into different dedupe partitions"
        );
    }

    #[ntex::test]
    async fn should_share_anonymous_partition_when_cookie_is_missing() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(100))
            .build()
            .start()
            .await;
        let router = start_router(&subgraphs).await;

        let (response_a, response_b) = futures::join!(
            router.send_graphql_request(QUERY, None, None),
            router.send_graphql_request(QUERY, None, None)
        );

        assert!(response_a.status().is_success());
        assert!(response_b.status().is_success());

        let products_requests = subgraphs
            .get_requests_log("products")
            .unwrap_or_default()
            .len();

        assert_eq!(
            products_requests, 1,
            "expected requests without a JWT cookie to share the default anonymous partition"
        );
    }
}
