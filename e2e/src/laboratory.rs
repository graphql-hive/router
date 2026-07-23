#[cfg(test)]
mod laboratory_e2e_tests {
    use crate::testkit::{Started, TestRouter, TestSubgraphs};

    /// The page inlines the Laboratory bundle and Monaco workers, putting it far above the
    /// client's default body limit.
    const MAX_PAGE_SIZE: usize = 128 * 1024 * 1024;

    /// Fetches the Laboratory page the way a browser would.
    async fn fetch_laboratory_html(router: &TestRouter<Started>) -> String {
        let res = router
            .serv()
            .get(router.graphql_path())
            .header(http::header::ACCEPT, "text/html")
            .send()
            .await
            .expect("failed to request the laboratory");

        assert_eq!(res.status(), 200);

        let body = res
            .body()
            .limit(MAX_PAGE_SIZE)
            .await
            .expect("failed to read the response body");

        String::from_utf8(body.to_vec()).expect("the laboratory page should be valid UTF-8")
    }

    async fn start_router(config: &str) -> TestRouter<Started> {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(config)
            .build()
            .start()
            .await
    }

    #[ntex::test]
    async fn serves_the_laboratory_unseeded_by_default() {
        let router = start_router(
            r#"
            laboratory:
              enabled: true
            "#,
        )
        .await;

        let html = fetch_laboratory_html(&router).await;

        // The placeholder must still be substituted, otherwise the page's JSON.parse throws on
        // every load and every user sees it in the console.
        assert!(
            !html.contains("__LABORATORY_PROPS__"),
            "the placeholder should always be replaced"
        );
        assert!(
            html.contains(r#"JSON.parse("{}")"#),
            "an unconfigured router should inject an empty seed"
        );
    }

    #[ntex::test]
    async fn injects_the_configured_preflight_script() {
        let router = start_router(
            r#"
            laboratory:
              enabled: true
              preflight:
                script: |
                  lab.request.headers.set('X-Env', 'staging');
            "#,
        )
        .await;

        let html = fetch_laboratory_html(&router).await;

        assert!(
            !html.contains("__LABORATORY_PROPS__"),
            "the placeholder should have been replaced"
        );
        assert!(
            html.contains("lab.request.headers.set('X-Env', 'staging');"),
            "the preflight script should be present in the page"
        );
        assert!(
            html.contains("\\\"enabled\\\":true"),
            "the preflight should be enabled"
        );
    }

    #[ntex::test]
    async fn injects_the_configured_operation_and_its_tab() {
        let router = start_router(
            r#"
            laboratory:
              enabled: true
              operations:
                - name: GetHello
                  query: |
                    query GetHello {
                      hello
                    }
                  headers: '{"X-Env": "staging"}'
            "#,
        )
        .await;

        let html = fetch_laboratory_html(&router).await;

        assert!(
            html.contains("router-seed:GetHello"),
            "the seeded operation should use a deterministic id"
        );
        assert!(
            html.contains("router-seed-tab:GetHello"),
            "the seeded operation should come with a matching tab"
        );
        assert!(
            html.contains("query GetHello"),
            "the seeded query should be present in the page"
        );
    }

    #[ntex::test]
    async fn does_not_serve_the_laboratory_when_it_is_disabled() {
        let router = start_router(
            r#"
            laboratory:
              enabled: false
              preflight:
                script: |
                  lab.request.headers.set('X-Env', 'staging');
            "#,
        )
        .await;

        // With the Laboratory disabled, content negotiation ignores the `text/html` preference
        // and the request falls through to regular GraphQL handling.
        let res = router
            .serv()
            .get(router.graphql_path())
            .header(http::header::ACCEPT, "text/html")
            .send()
            .await
            .expect("failed to request the laboratory");

        let body = res
            .body()
            .limit(MAX_PAGE_SIZE)
            .await
            .expect("failed to read the response body");
        let body = String::from_utf8(body.to_vec()).expect("should be valid UTF-8");

        assert!(
            !body.contains("lab.request.headers.set"),
            "the preflight script must not be served when the laboratory is disabled"
        );
    }
}
