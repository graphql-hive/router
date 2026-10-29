#[cfg(test)]
mod entity_reuse_e2e_tests {
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use ntex::http;
    use reqwest::StatusCode;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{
        some_header_map, ClientResponseExt, ResponseLike, Started, TestRouter, TestSubgraphs,
    };

    // In the bench operation, every product, user and inventory entity the last two waves ask
    // for was already fetched by `topProducts`, `users` and the first inventory fetch.
    // Reuse is on by default, so those fetches copy them instead of calling the subgraphs again.
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

        // Without reuse: accounts 2, inventory 2, products 2, reviews 1.
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

    // `inventory` at `users.@.reviews.@.product` copies from `inventory` at `topProducts.@`.
    // Reviews only point at products 1-4 and `topProducts` has 1-5, so all of them are there.
    const SHALLOW_AND_DEEP_INVENTORY: &str =
        "{ topProducts { upc inStock } users { reviews { product { upc inStock } } } }";

    async fn send_shallow_and_deep_inventory(
        subgraphs: &TestSubgraphs<Started>,
    ) -> sonic_rs::Value {
        let router = TestRouter::builder()
            .with_subgraphs(subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  traffic_shaping:
                    all:
                      dedupe_enabled: false
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(SHALLOW_AND_DEEP_INVENTORY, None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        res.json_body().await
    }

    // Every `inStock` under `users`, which is what the deep fetch fills.
    fn deep_in_stock(body: &sonic_rs::Value) -> Vec<String> {
        let mut values = Vec::new();
        for user in body["data"]["users"].as_array().unwrap().iter() {
            for review in user["reviews"].as_array().unwrap().iter() {
                let product = &review["product"];
                values.push(format!(
                    "{}: {}",
                    product["upc"].as_str().unwrap(),
                    product["inStock"]
                ));
            }
        }
        values
    }

    #[ntex::test]
    async fn deep_fetch_copies_everything_when_shallow_fetch_worked() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let body = send_shallow_and_deep_inventory(&subgraphs).await;

        assert!(body.get("errors").is_none(), "unexpected errors: {body}");
        insta::assert_snapshot!(deep_in_stock(&body).join("\n"), @r"
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        ");
        assert_eq!(
            subgraphs.get_requests_log("inventory").unwrap().len(),
            1,
            "the deep fetch should copy every product"
        );
    }

    // Once a fetch fails we stop copying. The failed fetch also leaves `inStock` out of the
    // response, so there'd be nothing to copy anyway. Either way, the deep fetch asks `inventory`.
    #[ntex::test]
    async fn deep_fetch_asks_subgraph_when_shallow_request_fails() {
        let inventory_calls = Arc::new(AtomicUsize::new(0));
        let subgraphs = TestSubgraphs::builder()
            .with_on_request({
                let inventory_calls = inventory_calls.clone();
                move |req| {
                    if req.path == "/inventory"
                        && inventory_calls.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        return Some(ResponseLike::new(
                            StatusCode::OK,
                            Some(r#"{"errors":[{"message":"inventory is down"}]}"#.to_string()),
                            some_header_map! { http::header::CONTENT_TYPE => "application/json" },
                        ));
                    }
                    None
                }
            })
            .build()
            .start()
            .await;
        let body = send_shallow_and_deep_inventory(&subgraphs).await;

        insta::assert_snapshot!(body["data"]["topProducts"].to_string(), @r#"[{"upc":"1","inStock":null},{"upc":"2","inStock":null},{"upc":"3","inStock":null},{"upc":"4","inStock":null},{"upc":"5","inStock":null}]"#);
        insta::assert_snapshot!(deep_in_stock(&body).join("\n"), @r"
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        ");
        assert_eq!(inventory_calls.load(Ordering::SeqCst), 2);
    }

    // With `topProducts` null there's nothing to copy from, so the deep fetch asks `inventory`.
    #[ntex::test]
    async fn deep_fetch_asks_subgraph_when_shallow_target_is_null() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                let asks_for_top_products = req
                    .body
                    .as_ref()
                    .is_some_and(|body| String::from_utf8_lossy(body).contains("topProducts"));
                if req.path == "/products" && asks_for_top_products {
                    return Some(ResponseLike::new(
                        StatusCode::OK,
                        Some(r#"{"data":{"topProducts":null}}"#.to_string()),
                        some_header_map! { http::header::CONTENT_TYPE => "application/json" },
                    ));
                }
                None
            })
            .build()
            .start()
            .await;
        let body = send_shallow_and_deep_inventory(&subgraphs).await;

        assert!(body.get("errors").is_none(), "unexpected errors: {body}");
        assert!(body["data"]["topProducts"].is_null());
        insta::assert_snapshot!(deep_in_stock(&body).join("\n"), @r"
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        1: true
        ");
        assert_eq!(
            subgraphs.get_requests_log("inventory").unwrap().len(),
            1,
            "only the deep fetch should call inventory"
        );
    }
}
