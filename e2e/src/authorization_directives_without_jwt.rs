#[cfg(test)]
mod authorization_directives_without_jwt_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    // `me` is `@authenticated` and `Product.price` is
    // `@requiresScopes(scopes: [["read:price"]])` in `supergraph-authz.graphql`.
    const QUERY: &str = "{ me { id name } topProducts(first: 1) { upc } }";

    async fn assert_me_is_refused(config: &str) {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                  source: file
                  path: supergraph-authz.graphql
                {config}
                "#
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let body = res.json_body_string_pretty().await;

        assert_eq!(
            subgraphs
                .get_requests_log("accounts")
                .map(|r| r.len())
                .unwrap_or(0),
            0,
            "an anonymous caller reached the subgraph behind an @authenticated field:\n{body}"
        );
        assert!(
            body.contains("UNAUTHORIZED_FIELD_OR_TYPE"),
            "expected an authorization error, got:\n{body}"
        );
        assert!(
            body.contains(r#""me": null"#),
            "expected `me` to be null:\n{body}"
        );
    }

    /// With no `jwt` block, nothing can authenticate a request, so every request is
    /// anonymous and an `@authenticated` field must be refused. Today the directives are
    /// skipped instead, and the field is served to anyone.
    #[ntex::test]
    async fn refuses_authenticated_field_when_jwt_is_not_configured() {
        assert_me_is_refused("").await;
    }

    /// Providers configured but `enabled` left off is the same situation, and the easiest
    /// one to ship by mistake: the config looks like authentication is set up.
    #[ntex::test]
    async fn refuses_authenticated_field_when_jwt_is_disabled() {
        assert_me_is_refused(
            r#"
                jwt:
                  enabled: false
                  jwks_providers:
                    - source: file
                      path: jwks.rsa512.json
                "#,
        )
        .await;
    }

    /// `@requiresScopes` on a nested field, without JWT: refused as well.
    #[ntex::test]
    async fn refuses_scoped_field_when_jwt_is_not_configured() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph-authz.graphql
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ topProducts(first: 1) { upc price } }", None, None)
            .await;
        let body = res.json_body_string_pretty().await;
        assert!(
            body.contains("UNAUTHORIZED_FIELD_OR_TYPE"),
            "a @requiresScopes field was served to an anonymous caller:\n{body}"
        );
    }

    /// The explicit opt-out still works: a deployment that wants the directives ignored
    /// says so. Passes before and after the fix.
    #[ntex::test]
    async fn serves_protected_fields_when_directives_are_explicitly_disabled() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph-authz.graphql
                authorization:
                  directives:
                    enabled: false
                "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        let body = res.json_body_string_pretty().await;
        assert!(!body.contains("UNAUTHORIZED_FIELD_OR_TYPE"), "{body}");
        assert!(!body.contains(r#""me": null"#), "{body}");
    }
}
