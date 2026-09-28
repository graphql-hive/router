//! End-to-end checks for runtime variable coercion (spec: "Coercing Variable Values" in the spec)

#[cfg(test)]
mod variables_coercion_e2e_tests {
    use futures::StreamExt;
    use hive_router::executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };
    use ntex::http;
    use reqwest::StatusCode;
    use serde_json::Value;
    use sonic_rs::{json, JsonValueTrait};

    use crate::testkit::{
        some_header_map, sort_json_keys, ClientResponseExt, ResponseLike, Started, TestRouter,
        TestSubgraphs,
    };

    const SUPERGRAPH: &str = "../bin/router/fixture/variables_coercion/supergraph.graphql";

    /// Starts a mock `test` subgraph that answers every request with empty data, and a router
    /// on the variable coercion supergraph, with `extra_config` appended to its config.
    async fn start(extra_config: &str) -> (TestSubgraphs<Started>, TestRouter<Started>) {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|_| {
                Some(ResponseLike::new(
                    StatusCode::OK,
                    Some(r#"{"data":{}}"#.to_string()),
                    some_header_map! { http::header::CONTENT_TYPE => "application/json" },
                ))
            })
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                "supergraph:\n  source: file\n  path: {SUPERGRAPH}\n{extra_config}"
            ))
            .build()
            .start()
            .await;
        (subgraphs, router)
    }

    /// The JSON bodies the `test` subgraph received, with keys sorted.
    fn subgraph_requests(subgraphs: &TestSubgraphs<Started>) -> Vec<Value> {
        subgraphs
            .get_requests_log("test")
            .unwrap_or_default()
            .iter()
            .map(|request| {
                let body = request.body.as_ref().expect("subgraph request has a body");
                sort_json_keys(serde_json::from_slice(body).expect("subgraph body is JSON"))
            })
            .collect()
    }

    /// The raw body of the only request the `test` subgraph received.
    fn only_subgraph_body(subgraphs: &TestSubgraphs<Started>) -> String {
        let requests = subgraphs.get_requests_log("test").unwrap_or_default();
        assert_eq!(requests.len(), 1, "expected exactly one subgraph request");
        String::from_utf8(requests[0].body.clone().unwrap().to_vec()).unwrap()
    }

    fn assert_no_subgraph_request(subgraphs: &TestSubgraphs<Started>) {
        assert_eq!(
            subgraph_requests(subgraphs),
            Vec::<Value>::new(),
            "expected no subgraph request"
        );
    }

    /// Percent-encodes a query string parameter value.
    fn encode(value: &str) -> String {
        value
            .bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (byte as char).to_string()
                }
                _ => format!("%{byte:02X}"),
            })
            .collect()
    }

    const MISSING_INT_MESSAGE: &str = r#"Variable "$v" has invalid value: Expected a value of non-null type "Int!" to be provided."#;

    /// spec §7.1 "Request Errors": no `data`, one error, and nothing is executed.
    #[ntex::test]
    async fn coercion_error_is_a_request_error() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request("query ($v: Int!) { int(input: $v) }", Some(json!({})), None)
            .await;

        assert_eq!(response.status(), 400);
        insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "errors": [
            {
              "extensions": {
                "code": "BAD_USER_INPUT"
              },
              "message": "Variable \"$v\" has invalid value: Expected a value of non-null type \"Int!\" to be provided."
            }
          ]
        }
        "#);
        assert_no_subgraph_request(&subgraphs);
    }

    /// GraphQL over HTTP: `application/graphql-response+json` gets a 400, `application/json` a
    /// 200, with the same body.
    #[ntex::test]
    async fn coercion_error_status_by_accept_header() {
        let (_subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                "query ($v: Int!) { int(input: $v) }",
                Some(json!({})),
                some_header_map! { http::header::ACCEPT => "application/json" },
            )
            .await;

        assert_eq!(response.status(), 200);
        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["message"].as_str(),
            Some(MISSING_INT_MESSAGE)
        );
        assert_eq!(
            body["errors"][0]["extensions"]["code"].as_str(),
            Some("BAD_USER_INPUT")
        );
        assert!(body.get("data").is_none());
    }

    /// spec §6.1: coercion happens before execution, so an invalid variable used by a later
    /// mutation field stops the whole operation, including the fields before it.
    #[ntex::test]
    async fn mutation_not_executed_when_a_later_variable_is_invalid() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                "mutation ($first: Color, $second: Color) { a: favoriteEnum(color: $first) b: favoriteEnum(color: $second) }",
                Some(json!({ "first": "RED", "second": "PURPLE" })),
                None,
            )
            .await;

        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["message"].as_str(),
            Some(
                r#"Variable "$second" has invalid value: Value "PURPLE" does not exist in "Color" enum."#
            )
        );
        assert_no_subgraph_request(&subgraphs);
    }

    /// spec §6.1.2: an absent variable is not sent (never as `null`), an explicit `null` is
    /// sent, and an absent variable with a default gets the default.
    #[ntex::test]
    async fn forwards_absent_null_and_default_correctly() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                "query ($absent: Int, $explicitNull: Int, $defaulted: Int = 5, $given: Int) { a: int(input: $absent) b: int(input: $explicitNull) c: int(input: $defaulted) d: int(input: $given) }",
                Some(json!({ "explicitNull": null, "given": 1 })),
                None,
            )
            .await;

        assert_eq!(response.status(), 200);
        let requests = subgraph_requests(&subgraphs);
        assert_eq!(requests.len(), 1);
        insta::assert_snapshot!(serde_json::to_string_pretty(&requests[0]["variables"]).unwrap(), @r#"
        {
          "defaulted": 5,
          "explicitNull": null,
          "given": 1
        }
        "#);
    }

    /// spec §6.1.2: coercion only covers the operation's variable definitions.
    #[ntex::test]
    async fn undeclared_variables_are_not_forwarded() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                "query ($v: String) { fieldWithNullableStringInput(input: $v) }",
                Some(json!({ "v": "a", "extra": { "x": [1] } })),
                None,
            )
            .await;

        assert_eq!(response.status(), 200);
        let requests = subgraph_requests(&subgraphs);
        assert_eq!(requests.len(), 1);
        insta::assert_snapshot!(serde_json::to_string(&requests[0]["variables"]).unwrap(), @r#"{"v":"a"}"#);
    }

    /// Input objects reach the subgraph as sent: field defaults are not injected (the subgraph
    /// applies them), explicit `null`s are kept, and so is the key order.
    #[ntex::test]
    async fn input_object_forwarded_as_sent() {
        let (subgraphs, router) = start("").await;

        // Sent as raw text, so the key order is exactly the one written here.
        let response = router
            .serv()
            .post(router.graphql_path())
            .header(http::header::CONTENT_TYPE, "application/json")
            .send_body(
                r#"{"query":"query ($defaults: DefaultSevenInput, $example: ExampleInputObject) { defaultSeven(input: $defaults) exampleInput(input: $example) }","variables":{"defaults":{},"example":{"b":1,"a":null}}}"#,
            )
            .await
            .expect("failed to send request");

        assert_eq!(response.status(), 200);
        let body = only_subgraph_body(&subgraphs);
        assert!(
            body.contains(r#""defaults":{}"#),
            "field default injected: {body}"
        );
        assert!(
            body.contains(r#""example":{"b":1,"a":null}"#),
            "input object changed: {body}"
        );
    }

    #[ntex::test]
    async fn invalid_variable_rejected_over_get() {
        let (subgraphs, router) = start("").await;

        let response = router
            .serv()
            .get(format!(
                "/graphql?query={}&variables={}",
                encode("query ($v: Int!) { int(input: $v) }"),
                encode("{}")
            ))
            .send()
            .await
            .expect("failed to send GET request");

        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["message"].as_str(),
            Some(MISSING_INT_MESSAGE)
        );
        assert_eq!(
            body["errors"][0]["extensions"]["code"].as_str(),
            Some("BAD_USER_INPUT")
        );
        assert_no_subgraph_request(&subgraphs);
    }

    /// spec §6.1.2: no variables is an empty map, so the default applies. POST already treats
    /// `"variables": null` that way.
    #[ntex::test]
    async fn get_variables_null_is_an_empty_map() {
        let (subgraphs, router) = start("").await;

        let response = router
            .serv()
            .get(format!(
                "/graphql?query={}&variables=null",
                encode("query ($v: Int = 5) { int(input: $v) }")
            ))
            .send()
            .await
            .expect("failed to send GET request");

        assert_eq!(response.status(), 200, "{}", response.string_body().await);
        let requests = subgraph_requests(&subgraphs);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            serde_json::to_string(&requests[0]["variables"]).unwrap(),
            r#"{"v":5}"#
        );
    }

    #[ntex::test]
    async fn invalid_variable_rejected_over_websocket() {
        let (subgraphs, router) = start("websocket:\n  enabled: true\n").await;

        let mut client = WsClient::new(router.ws().await)
            .init(None)
            .await
            .expect("failed to init WsClient");
        let mut stream = client
            .subscribe(
                SubscribePayload {
                    query: "query ($v: Int!) { int(input: $v) }".to_string(),
                    ..Default::default()
                },
                None,
            )
            .await
            .expect("failed to subscribe");

        let response = stream
            .next()
            .await
            .expect("expected a response")
            .expect("WebSocket response failed");
        let errors = response.errors.expect("expected GraphQL errors");
        assert_eq!(errors[0].message, MISSING_INT_MESSAGE);
        assert_eq!(errors[0].extensions.code.as_deref(), Some("BAD_USER_INPUT"));
        assert!(
            stream.next().await.is_none(),
            "expected the stream to complete"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    #[ntex::test]
    async fn invalid_subscription_variable_rejected_over_websocket() {
        let (subgraphs, router) =
            start("websocket:\n  enabled: true\nsubscriptions:\n  enabled: true\n").await;

        let mut client = WsClient::new(router.ws().await)
            .init(None)
            .await
            .expect("failed to init WsClient");
        let mut variables = std::collections::HashMap::new();
        variables.insert("color".to_string(), json!("PURPLE"));
        let mut stream = client
            .subscribe(
                SubscribePayload {
                    query: "subscription ($color: Color!) { subscribeToEnum(color: $color) }"
                        .to_string(),
                    variables: Some(variables),
                    ..Default::default()
                },
                None,
            )
            .await
            .expect("failed to subscribe");

        let response = stream
            .next()
            .await
            .expect("expected a response")
            .expect("WebSocket response failed");
        let errors = response.errors.expect("expected GraphQL errors");
        assert_eq!(
            errors[0].message,
            r#"Variable "$color" has invalid value: Value "PURPLE" does not exist in "Color" enum."#
        );
        assert_eq!(errors[0].extensions.code.as_deref(), Some("BAD_USER_INPUT"));
        assert!(
            stream.next().await.is_none(),
            "expected the stream to complete"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    #[ntex::test]
    async fn invalid_variable_rejected_over_sse() {
        let (subgraphs, router) = start("subscriptions:\n  enabled: true\n").await;

        let response = router
            .send_graphql_request(
                "subscription ($color: Color!) { subscribeToEnum(color: $color) }",
                Some(json!({ "color": "PURPLE" })),
                some_header_map! { http::header::ACCEPT => "text/event-stream" },
            )
            .await;

        let body = response.string_body().await;
        assert!(
            body.contains(r#"Value \"PURPLE\" does not exist in \"Color\" enum."#)
                && body.contains("BAD_USER_INPUT"),
            "unexpected body: {body}"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    #[ntex::test]
    async fn invalid_variable_rejected_over_multipart() {
        let (subgraphs, router) = start("subscriptions:\n  enabled: true\n").await;

        let response = router
            .send_graphql_request(
                "subscription ($color: Color!) { subscribeToEnum(color: $color) }",
                Some(json!({ "color": "PURPLE" })),
                some_header_map! {
                    http::header::ACCEPT => "multipart/mixed;subscriptionSpec=1.0"
                },
            )
            .await;

        let body = response.string_body().await;
        assert!(
            body.contains(r#"Value \"PURPLE\" does not exist in \"Color\" enum."#)
                && body.contains("BAD_USER_INPUT"),
            "unexpected body: {body}"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    #[ntex::test]
    async fn invalid_variable_rejected_for_persisted_document() {
        let manifest = tempfile::NamedTempFile::new().expect("failed to create manifest");
        std::fs::write(
            manifest.path(),
            r#"{"sha256:int": "query ($v: Int!) { int(input: $v) }"}"#,
        )
        .expect("failed to write manifest");
        let (subgraphs, router) = start(&format!(
            "persisted_documents:\n  enabled: true\n  require_id: true\n  storage:\n    type: file\n    path: \"{}\"\n",
            manifest.path().display()
        ))
        .await;

        let response = router
            .send_post_request(
                router.graphql_path(),
                json!({ "documentId": "sha256:int", "variables": {} }),
                None,
            )
            .await;

        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["message"].as_str(),
            Some(MISSING_INT_MESSAGE)
        );
        assert_eq!(
            body["errors"][0]["extensions"]["code"].as_str(),
            Some("BAD_USER_INPUT")
        );
        assert_no_subgraph_request(&subgraphs);
    }

    /// `variables` must be a JSON object (or `null`); anything else is a malformed request.
    #[ntex::test]
    async fn variables_not_an_object_is_bad_request() {
        let (subgraphs, router) = start("").await;

        for variables in [json!([]), json!("x"), json!(1)] {
            let response = router
                .send_post_request(
                    router.graphql_path(),
                    json!({
                        "query": "query ($v: String) { fieldWithNullableStringInput(input: $v) }",
                        "variables": variables,
                    }),
                    None,
                )
                .await;

            assert_eq!(response.status(), 400);
            let body = response.json_body().await;
            assert_eq!(
                body["errors"][0]["extensions"]["code"].as_str(),
                Some("BAD_REQUEST"),
                "unexpected body: {body}"
            );
        }
        assert_no_subgraph_request(&subgraphs);
    }

    /// An invalid document is a validation error, even when the problem is a variable default.
    #[ntex::test]
    async fn validation_runs_before_coercion() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                r#"query ($v: Int = "x") { int(input: $v) }"#,
                Some(json!({})),
                None,
            )
            .await;

        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["extensions"]["code"].as_str(),
            Some("ValuesOfCorrectType"),
            "unexpected body: {body}"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    /// `@skip`/`@include` only see coerced values: an invalid one is rejected before planning,
    /// a default is applied, and `null` does not include the field (spec §6.3.2).
    #[ntex::test]
    async fn skip_include_see_coerced_values() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_graphql_request(
                "query ($v: Boolean!) { fieldWithNullableStringInput @include(if: $v) int }",
                Some(json!({ "v": "false" })),
                None,
            )
            .await;
        let body = response.json_body().await;
        assert_eq!(
            body["errors"][0]["message"].as_str(),
            Some(
                r#"Variable "$v" has invalid value: Boolean cannot represent a non boolean value: "false""#
            )
        );
        assert_no_subgraph_request(&subgraphs);

        let response = router
            .send_graphql_request(
                "query ($defaultFalse: Boolean = false, $givenNull: Boolean = true) { a: fieldWithNullableStringInput @include(if: $defaultFalse) b: fieldWithNullableStringInput @include(if: $givenNull) int }",
                Some(json!({ "givenNull": null })),
                None,
            )
            .await;
        assert_eq!(response.status(), 200);
        insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "int": null
          }
        }
        "#);
        let requests = subgraph_requests(&subgraphs);
        assert_eq!(requests.len(), 1);
        insta::assert_snapshot!(serde_json::to_string(&requests[0]["variables"]).unwrap(), @r#"{"defaultFalse":false,"givenNull":null}"#);
    }

    /// spec §6.1: the operation is selected first, then its variables are coerced.
    #[ntex::test]
    async fn operation_selection_error_before_coercion() {
        let (subgraphs, router) = start("").await;

        let response = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "query A($a: Int!) { int(input: $a) } query B($b: Int!) { int(input: $b) }",
                    "operationName": "C",
                    "variables": {},
                }),
                None,
            )
            .await;

        let body = response.json_body().await;
        assert_ne!(
            body["errors"][0]["extensions"]["code"].as_str(),
            Some("BAD_USER_INPUT"),
            "unexpected body: {body}"
        );
        assert_no_subgraph_request(&subgraphs);
    }

    /// Demand control reads slicing arguments from variables, so it must only ever see valid
    /// ones.
    #[ntex::test]
    async fn demand_control_sees_validated_variables() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph_demand_control.graphql
                demand_control:
                  enabled: true
                  operation_cost:
                    max: 100
                    mode: enforce
                  subgraphs_budget:
                    mode: enforce
                "#,
            )
            .build()
            .start()
            .await;

        for (limit, message) in [
            (
                json!(2147483648_u64),
                r#"Variable "$limit" has invalid value: Int cannot represent non 32-bit signed integer value: 2147483648"#,
            ),
            (
                json!(10.0),
                r#"Variable "$limit" has invalid value: Int cannot represent non-integer value: 10.0"#,
            ),
        ] {
            let response = router
                .send_graphql_request(
                    "query ($limit: Int!) { newestAdditions(limit: $limit) { title } }",
                    Some(json!({ "limit": limit })),
                    None,
                )
                .await;
            let body = response.json_body().await;
            assert_eq!(body["errors"][0]["message"].as_str(), Some(message));
            assert_eq!(
                body["errors"][0]["extensions"]["code"].as_str(),
                Some("BAD_USER_INPUT")
            );
        }
    }
}
