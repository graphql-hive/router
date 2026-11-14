#[cfg(test)]
mod entity_reuse_e2e_tests {
    use std::collections::BTreeMap;

    use ntex::http;
    use reqwest::StatusCode;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{
        some_header_map, ClientResponseExt, ResponseLike, Started, TestRouter, TestSubgraphs,
    };

    // In the bench operation, every product and user entity the deep calls ask for was already
    // fetched by `topProducts` and `users`, which they wait for. With reuse on, those calls copy
    // them instead of calling the subgraphs again.
    #[ntex::test]
    async fn bench_operation_copies_entities_it_already_has() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  traffic_shaping:
                    experimental_entity_reuse: true
                    all:
                      dedupe_enabled: false
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(include_str!("../../bench/operation.graphql"), None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let body: sonic_rs::Value = res.json_body().await;
        let expected: sonic_rs::Value =
            sonic_rs::from_str(include_str!("../../bench/expected_response.json")).unwrap();
        assert_eq!(
            body, expected,
            "copied entities have to give the same response as fetched ones"
        );

        let mut subgraph_request_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for subgraph in ["accounts", "inventory", "products", "reviews"] {
            let request_count = subgraphs
                .get_requests_log(subgraph)
                .map_or(0, |requests| requests.len());
            subgraph_request_counts.insert(subgraph, request_count);
        }

        // Without reuse: accounts 2, inventory 2, products 2, reviews 1. The deep `inventory` call
        // waits for the one at `topProducts` and copies everything from it.
        insta::assert_snapshot!(
            sonic_rs::to_string_pretty(&subgraph_request_counts).unwrap(),
            @r#"
        {
          "accounts": 1,
          "inventory": 1,
          "products": 1,
          "reviews": 1
        }
        "#
        );
    }

    // The deep `products` call goes after `reviews`, which goes after `topProducts`, so it
    // copies names from `topProducts`.
    const SHALLOW_AND_DEEP_PRODUCTS: &str =
        "{ topProducts { upc name reviews { author { reviews { product { upc name } } } } } }";

    async fn send(subgraphs: &TestSubgraphs<Started>, query: &str) -> sonic_rs::Value {
        let router = TestRouter::builder()
            .with_subgraphs(subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  traffic_shaping:
                    experimental_entity_reuse: true
                    all:
                      dedupe_enabled: false
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(query, None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");
        res.json_body().await
    }

    fn deep_names(body: &sonic_rs::Value) -> Vec<String> {
        let mut names = Vec::new();
        for product in body["data"]["topProducts"].as_array().unwrap().iter() {
            for review in product["reviews"].as_array().unwrap().iter() {
                for review in review["author"]["reviews"].as_array().unwrap().iter() {
                    let product = &review["product"];
                    names.push(format!(
                        "{}: {}",
                        product["upc"].as_str().unwrap(),
                        product["name"]
                    ));
                }
            }
        }
        names
    }

    fn products_requests(subgraphs: &TestSubgraphs<Started>) -> usize {
        subgraphs
            .get_requests_log("products")
            .map_or(0, |requests| requests.len())
    }

    // Every review in the test data is of product 1, and `topProducts` has it.
    #[ntex::test]
    async fn deep_fetch_copies_what_it_already_has() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let body = send(&subgraphs, SHALLOW_AND_DEEP_PRODUCTS).await;

        assert!(body.get("errors").is_none(), "unexpected errors: {body}");
        let names = deep_names(&body);
        assert!(!names.is_empty());
        assert!(
            names.iter().all(|name| name == r#"1: "Table""#),
            "{names:?}"
        );
        assert_eq!(products_requests(&subgraphs), 1, "only `topProducts` asks");
    }

    // `topProducts` gets product 1's `name` as `null` with an error. Copying that `null` would
    // give the deep product 1 a `null` with no error at its path, so once an error came back we
    // stop copying and the deep call asks `products`.
    #[ntex::test]
    async fn deep_fetch_asks_subgraph_when_a_field_is_null_from_an_error() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                let asks_for_top_products = req
                    .body
                    .as_ref()
                    .is_some_and(|body| String::from_utf8_lossy(body).contains("topProducts"));
                if req.path == "/products" && asks_for_top_products {
                    return Some(ResponseLike::new(
                        StatusCode::OK,
                        Some(
                            r#"{"data":{"topProducts":[{"__typename":"Product","upc":"1","name":null},{"__typename":"Product","upc":"2","name":"Couch"}]},"errors":[{"message":"name lookup failed","path":["topProducts",0,"name"]}]}"#
                                .to_string(),
                        ),
                        some_header_map! { http::header::CONTENT_TYPE => "application/json" },
                    ));
                }
                None
            })
            .build()
            .start()
            .await;
        let body = send(&subgraphs, SHALLOW_AND_DEEP_PRODUCTS).await;

        insta::assert_snapshot!(body["errors"].to_string(), @r#"[{"message":"Unexpected error","path":["topProducts",0,"name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"products"}}]"#);
        let names = deep_names(&body);
        assert!(!names.is_empty());
        assert!(
            names.iter().all(|name| name == r#"1: "Table""#),
            "{names:?}"
        );
        assert_eq!(products_requests(&subgraphs), 2);
    }
}
