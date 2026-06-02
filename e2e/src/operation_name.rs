#[cfg(test)]
mod operation_name_e2e_tests {
    use ntex::client::ClientResponse;
    use serde_json::Value;
    use sonic_rs::{json, to_string_pretty};

    use crate::testkit::{ClientResponseExt, RequestLike, Started, TestRouter, TestSubgraphs};

    fn request_body_json(request: &RequestLike) -> Value {
        let body = request
            .body
            .as_ref()
            .expect("expected subgraph request body to be present");
        serde_json::from_slice(body).unwrap_or_else(|err| {
            panic!(
                "expected subgraph request body to be valid JSON: {err}; body={}",
                String::from_utf8_lossy(body)
            )
        })
    }

    fn first_request_from(subgraphs: &TestSubgraphs<Started>, subgraph: &str) -> Value {
        let requests = subgraphs
            .get_requests_log(subgraph)
            .unwrap_or_else(|| panic!("expected requests sent to {subgraph} subgraph"));

        assert_eq!(
            requests.len(),
            1,
            "expected exactly 1 request to {subgraph} subgraph"
        );

        request_body_json(&requests[0])
    }

    async fn assert_success(response: ClientResponse) {
        let status = response.status();
        let body = response.string_body().await;

        assert!(
            status.is_success(),
            "Expected 2xx response, got {status}: {body}"
        );
    }

    #[ntex::test]
    async fn does_not_forward_operation_name_by_default() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("query GetMe { me { id } }", None, None)
            .await;

        assert_success(response).await;

        let accounts_body = first_request_from(&subgraphs, "accounts");

        insta::assert_snapshot!(
            to_string_pretty(&accounts_body).unwrap(),
            @r#"
            {
              "query": "{me{id}}"
            }
            "#
        );
    }

    #[ntex::test]
    async fn forwards_parsed_document_operation_name_when_enabled_globally() {
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
                    forward_operation_name: true
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "query GetUser($id: ID!) { user(id: $id) { id name } }",
                    "variables": { "id": "1" },
                }),
                None,
            )
            .await;

        assert_success(response).await;

        let accounts_body = first_request_from(&subgraphs, "accounts");
        insta::assert_snapshot!(
            to_string_pretty(&accounts_body).unwrap(),
            @r#"
            {
              "operationName": "GetUser__2",
              "query": "query GetUser__2($id:ID!){user(id: $id){id name}}",
              "variables": {
                "id": "1"
              }
            }
            "#
        );
    }

    #[ntex::test]
    async fn forwards_selected_transport_operation_name() {
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
                    forward_operation_name: true
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "query First { me { id } } query Second { user(id: \"1\") { id } }",
                    "operationName": "Second",
                }),
                None,
            )
            .await;

        assert_success(response).await;

        let accounts_body = first_request_from(&subgraphs, "accounts");
        insta::assert_snapshot!(
            to_string_pretty(&accounts_body).unwrap(),
            @r#"
            {
              "operationName": "Second__2",
              "query": "query Second__2 {user(id: \"1\"){id}}"
            }
            "#
        );
    }

    #[ntex::test]
    async fn enables_forwarding_for_selected_subgraph_only() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                traffic_shaping:
                  subgraphs:
                    accounts:
                      forward_operation_name: true
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "query GetMeReviews { me { id reviews { body } } }",
                None,
                None,
            )
            .await;

        assert_success(response).await;

        let accounts_body = first_request_from(&subgraphs, "accounts");
        insta::assert_snapshot!(
            to_string_pretty(&accounts_body).unwrap(),
            @r#"
            {
              "operationName": "GetMeReviews__2",
              "query": "query GetMeReviews__2 {me{__typename id}}"
            }
            "#
        );

        let reviews_body = first_request_from(&subgraphs, "reviews");
        insta::assert_snapshot!(
            to_string_pretty(&reviews_body).unwrap(),
            @r#"
            {
              "query": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{body}}}}",
              "variables": {
                "representations": [
                  {
                    "__typename": "User",
                    "id": "1"
                  }
                ]
              }
            }
            "#
        );
    }

    #[ntex::test]
    async fn disables_forwarding_for_selected_subgraph_override() {
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
                    forward_operation_name: true
                  subgraphs:
                    accounts:
                      forward_operation_name: false
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "query GetMeReviews { me { id reviews { body } } }",
                None,
                None,
            )
            .await;

        assert_success(response).await;

        let accounts_body = first_request_from(&subgraphs, "accounts");
        insta::assert_snapshot!(
            to_string_pretty(&accounts_body).unwrap(),
            @r#"
            {
              "query": "{me{__typename id}}"
            }
            "#
        );

        let reviews_body = first_request_from(&subgraphs, "reviews");
        insta::assert_snapshot!(
            to_string_pretty(&reviews_body).unwrap(),
            @r#"
            {
              "operationName": "GetMeReviews__3",
              "query": "query GetMeReviews__3($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{body}}}}",
              "variables": {
                "representations": [
                  {
                    "__typename": "User",
                    "id": "1"
                  }
                ]
              }
            }
            "#
        );
    }

    #[ntex::test]
    async fn forwards_named_mutation_to_products() {
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
                    forward_operation_name: true
                "#,
            )
            .build()
            .start()
            .await;

        let response = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "mutation SaveOneof($input: OneOfTestInput!) { oneofTest(input: $input) { string } }",
                    "variables": { "input": { "string": "hello" } },
                }),
                None,
            )
            .await;

        assert_success(response).await;

        let products_body = first_request_from(&subgraphs, "products");
        insta::assert_snapshot!(
            to_string_pretty(&products_body).unwrap(),
            @r#"
            {
              "operationName": "SaveOneof__2",
              "query": "mutation SaveOneof__2($input:OneOfTestInput!){oneofTest(input: $input){string}}",
              "variables": {
                "input": {
                  "string": "hello"
                }
              }
            }
            "#
        );
    }
}
