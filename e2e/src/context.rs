#[cfg(test)]
mod context_e2e_tests {
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use crate::testkit::{ClientResponseExt, Started, TestRouter};

    fn body(request: &mockito::Request) -> Value {
        serde_json::from_slice(request.body().unwrap()).unwrap()
    }

    async fn start_router(fixture: &str, server: &mockito::Server) -> TestRouter<Started> {
        start_router_with_config(fixture, server, "").await
    }

    async fn start_router_with_config(
        fixture: &str,
        server: &mockito::Server,
        config: &str,
    ) -> TestRouter<Started> {
        let host = server.host_with_port();
        TestRouter::builder()
            .inline_config(format!(
                r#"
            supergraph:
              source: file
              path: ../bin/router/fixture/tests/{fixture}.supergraph.graphql
            override_subgraph_urls:
              subgraphs:
                a:
                  url: "http://{host}/a"
                b:
                  url: "http://{host}/b"
            {config}
            "#
            ))
            .build()
            .start()
            .await
    }

    #[ntex::test]
    async fn chunks_context_groups_into_aliased_requests() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        // one more group than fits in one request
        let users: Vec<_> = (0..16)
            .map(|index| {
                json!({
                    "__typename": "User",
                    "currency": index.to_string(),
                    "txs": [{"__typename": "Transaction", "id": "same"}]
                })
            })
            .collect();
        let root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"data": {"users": users}}).to_string())
            .expect(1)
            .create_async()
            .await;
        let aliased_data: serde_json::Map<_, _> = (0..15)
            .map(|index| (format!("_entities_{index}"), json!([{"amount": index}])))
            .collect();
        let aliased = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["query"].as_str().unwrap().contains("_entities_14:")
                    && !body["query"].as_str().unwrap().contains("_entities_15:")
                    && (0..15).all(|index| {
                        body["variables"][format!("__context_0_{index}")] == index.to_string()
                            && body["variables"][format!("representations_{index}")]
                                == json!([{"__typename": "Transaction", "id": "same"}])
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"data": aliased_data}).to_string())
            .expect(1)
            .create_async()
            .await;
        let single = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]
                    == json!({
                        "__context_0": "15",
                        "representations": [{"__typename": "Transaction", "id": "same"}]
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":15}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request("{ users { txs { amount } } }", None, None)
            .await;
        let response: Value = serde_json::from_str(&res.json_body_string_pretty().await).unwrap();
        let expected: Vec<_> = (0..16)
            .map(|index| json!({"txs": [{"amount": index}]}))
            .collect();
        assert_eq!(response, json!({"data": {"users": expected}}));
        root.assert_async().await;
        aliased.assert_async().await;
        single.assert_async().await;
    }

    #[ntex::test]
    async fn max_context_aliases_per_request_is_configurable_per_subgraph() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router_with_config(
            "context-basic",
            &server,
            r#"
            traffic_shaping:
              subgraphs:
                a:
                  max_context_aliases_per_request: 2
            "#,
        )
        .await;
        let root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[
                {"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]},
                {"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]},
                {"__typename":"User","currency":"GBP","txs":[{"__typename":"Transaction","id":"same"}]}
            ]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let aliased = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                !body["query"].as_str().unwrap().contains("_entities_2:")
                    && body["variables"]["__context_0_0"] == "USD"
                    && body["variables"]["__context_0_1"] == "EUR"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities_0":[{"amount":1}],"_entities_1":[{"amount":2}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let single = server
            .mock("POST", "/a")
            .match_request(|r| body(r)["variables"]["__context_0"] == "GBP")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":3}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request("{ users { txs { amount } } }", None, None)
            .await;
        let response: Value = serde_json::from_str(&res.json_body_string_pretty().await).unwrap();
        assert_eq!(
            response,
            json!({"data": {"users": [
                {"txs": [{"amount": 1}]},
                {"txs": [{"amount": 2}]},
                {"txs": [{"amount": 3}]}
            ]}})
        );
        root.assert_async().await;
        aliased.assert_async().await;
        single.assert_async().await;
    }

    #[ntex::test]
    async fn dedupes_entities_with_equal_context_values() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[
                {"__typename":"User","currency":"USD","txs":[
                    {"__typename":"Transaction","id":"same"},
                    {"__typename":"Transaction","id":"same"}
                ]},
                {"__typename":"User","currency":"USD","txs":[
                    {"__typename":"Transaction","id":"same"},
                    {"__typename":"Transaction","id":"same"}
                ]}
            ]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
                    && body["variables"]["representations"]
                        == json!([{"__typename":"Transaction","id":"same"}])
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ users { txs { amount } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {
                "amount": 1
              },
              {
                "amount": 1
              }
            ]
          },
          {
            "txs": [
              {
                "amount": 1
              },
              {
                "amount": 1
              }
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn groups_entities_by_context_value_including_null() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[
                {"__typename":"User","currency":"USD","txs":[
                    {"__typename":"Transaction","id":"same"},
                    {"__typename":"Transaction","id":"same"}
                ]},
                {"__typename":"User","currency":"EUR","txs":[
                    {"__typename":"Transaction","id":"same"},
                    {"__typename":"Transaction","id":"same"}
                ]},
                {"__typename":"User","currency":null,"txs":[
                    {"__typename":"Transaction","id":"same"},
                    {"__typename":"Transaction","id":"same"}
                ]}
            ]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0_0"] == "USD"
                    && body["variables"]["__context_0_1"] == "EUR"
                    && body["variables"].get("__context_0_2") == Some(&Value::Null)
                    && (0..3).all(|index| {
                        body["variables"][format!("representations_{index}")]
                            == json!([{"__typename":"Transaction","id":"same"}])
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"_entities_0":[{"amount":1}],"_entities_1":[{"amount":2}],"_entities_2":[{"amount":0}]}}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ users { txs { amount } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {
                "amount": 1
              },
              {
                "amount": 1
              }
            ]
          },
          {
            "txs": [
              {
                "amount": 2
              },
              {
                "amount": 2
              }
            ]
          },
          {
            "txs": [
              {
                "amount": 0
              },
              {
                "amount": 0
              }
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn nested_ancestor_picks_nearest() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[{"child":{
                "__typename":"User",
                "currency":"USD",
                "txs":[{"__typename":"Transaction","id":"1"}]
            }}]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ users { child { txs { amount } } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "child": {
              "txs": [
                {
                  "amount": 1
                }
              ]
            }
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn ancestor_through_non_context_wrapper() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[{
                "__typename":"User",
                "currency":"USD",
                "wrapper":{"txs":[{"__typename":"Transaction","id":"1"}]}
            }]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ users { wrapper { txs { amount } } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "wrapper": {
              "txs": [
                {
                  "amount": 1
                }
              ]
            }
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn interface_context() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"account":{
                "__typename":"User",
                "currency":"USD",
                "txs":[{"__typename":"Transaction","id":"1"}]
            }}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"fromInterface":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ account { txs { fromInterface } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "account": {
          "txs": [
            {
              "fromInterface": 1
            }
          ]
        }
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn union_context_with_type_condition() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"viewer":{
                "__typename":"Other",
                "currency":"EUR",
                "txs":[{"__typename":"Transaction","id":"1"}]
            }}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "EUR"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"fromUnion":2}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ viewer { ... on Other { txs { fromUnion } } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "viewer": {
          "txs": [
            {
              "fromUnion": 2
            }
          ]
        }
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn multiple_arguments_from_different_ancestors() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-outer", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"outer":{
                "__typename":"Outer",
                "code":"outer",
                "user":{
                    "__typename":"User",
                    "currency":"USD",
                    "txs":[{"__typename":"Transaction","id":"1"}]
                }
            }}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
                    && body["variables"]["__context_1"] == "outer"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"multiple":11}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(r#"{ outer { user { txs { multiple } } } }"#, None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "outer": {
          "user": {
            "txs": [
              {
                "multiple": 11
              }
            ]
          }
        }
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn requires_and_context_on_one_field() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[{
                "__typename":"User",
                "currency":"USD",
                "txs":[{"__typename":"Transaction","id":"1"}]
            }]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let base = server
            .mock("POST", "/b")
            .match_request(|r| {
                let body = body(r);
                body["query"].as_str().unwrap().contains("base")
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"base":10}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_2"] == "USD"
                    && body["variables"]["__context_0"] == 7
                    && body["query"]
                        .as_str()
                        .unwrap()
                        .contains("multiplier: $__context_0")
                    && body["query"]
                        .as_str()
                        .unwrap()
                        .contains("code: $__context_2")
                    && body["variables"]["representations"]
                        == json!([{"__typename":"Transaction","base":10,"id":"1"}])
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"required":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
        .send_graphql_request(
            "query($__context_0: Int = 7, $__context_1: Boolean! = true) { users { txs { required(multiplier: $__context_0) @include(if: $__context_1) } } }",
            None,
            None,
        )
        .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {
                "required": 1
              }
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        base.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn client_alias_on_ancestor_and_colliding_with_context_key() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"renamed":[{
                "__typename":"User",
                "currency":"user-id",
                "_internal_qp_alias_0":"USD",
                "txs":[{"__typename":"Transaction","id":"1"}]
            }]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "USD"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ renamed: users { currency: id txs { amount } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "renamed": [
          {
            "currency": "user-id",
            "txs": [
              {
                "amount": 1
              }
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn skipped_context_field_sends_no_entity_request() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"users":[{"txs":[{"__typename":"Transaction","id":"1"}]}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["query"].as_str().unwrap().contains("_entities")
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[]}}"#)
            .expect(0)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ users { txs { amount @skip(if: true) } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {}
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn cross_subgraph_merges_into_ancestor_fetch() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-cross", &server).await;
        let _root = server
        .mock("POST", "/a")
        .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"data":{"user":{"__typename":"User","id":"1","userCurrency":{"isoCode":"EUR"}}}}"#,
        )
        .expect(1)
        .create_async()
        .await;
        let entities = server
            .mock("POST", "/b")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0"] == "EUR"
                    && body["query"].as_str().unwrap().contains("...on User")
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"lastTransaction":{"amountInUserCurrency":2}}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ user { lastTransaction { amountInUserCurrency } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "user": {
          "lastTransaction": {
            "amountInUserCurrency": 2
          }
        }
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn batch_aliases_with_unequal_group_counts() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
        .mock("POST", "/a")
        .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"data":{
                "users":[
                    {"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]},
                    {"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]}
                ],
                "secondUsers":[
                    {"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]}
                ]
            }}"#,
        )
        .expect(1)
        .create_async()
        .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["variables"]["__context_0_0"] == "USD"
                    && body["variables"]["__batch_reps_0_0"]
                        == json!([{"__typename":"Transaction","id":"same"}])
                    && body["variables"]["__context_0_1"] == "EUR"
                    && body["variables"]["__batch_reps_0_1"] == json!([])
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"_e0_0":[{"amount":1}],"_e1_0":[{"amount":1}],"_e0_1":[],"_e1_1":[{"amount":2}]}}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ users { txs { amount } } secondUsers { txs { amount } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {
                "amount": 1
              }
            ]
          },
          {
            "txs": [
              {
                "amount": 2
              }
            ]
          }
        ],
        "secondUsers": [
          {
            "txs": [
              {
                "amount": 1
              }
            ]
          }
        ]
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn batch_alias_without_entities_sends_empty_representations() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let _root = server
        .mock("POST", "/a")
        .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"data":{
                "users":[
                    {"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]},
                    {"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]}
                ],
                "secondUsers":[]
            }}"#,
        )
        .expect(1)
        .create_async()
        .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["query"].as_str().unwrap().contains("_e0_0")
                    && (0..2).all(|wave| {
                        body["variables"][format!("__batch_reps_0_{wave}")] == json!([])
                            && body["variables"].get(format!("__context_1_{wave}"))
                                == Some(&Value::Null)
                    })
                    && body["variables"]["__context_0_0"] == "USD"
                    && body["variables"]["__context_0_1"] == "EUR"
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_e1_0":[{"amount":1}],"_e1_1":[{"amount":2}]}}"#)
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request(
                r#"{ users { txs { amount } } secondUsers { txs { amount } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
    {
      "data": {
        "users": [
          {
            "txs": [
              {
                "amount": 1
              }
            ]
          },
          {
            "txs": [
              {
                "amount": 2
              }
            ]
          }
        ],
        "secondUsers": []
      }
    }
    "#);
        _root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn dynamic_skip_and_include_preserve_context_and_client_variables() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        for (include, skip) in [(false, false), (false, true), (true, false), (true, true)] {
            let root = server
                .mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    r#"{"data":{"users":[{
                    "id":"u",
                    "__typename":"User",
                    "currency":"USD",
                    "txs":[{"__typename":"Transaction","id":"1"}]
                }]}}"#,
                )
                .expect(1)
                .create_async()
                .await;
            let entities = server
                .mock("POST", "/a")
                .match_request(|r| body(r)["variables"]["__context_2"] == "USD")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
                .expect(usize::from(include && !skip))
                .create_async()
                .await;
            let response = router.send_graphql_request(
            "query($__context_0: Boolean!, $__context_1: Boolean!) { users { id txs { amount @include(if: $__context_0) @skip(if: $__context_1) } } }",
            Some(sonic_rs::json!({"__context_0": include, "__context_1": skip})),
            None,
        ).await;
            let response: Value =
                serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
            let transaction = if include && !skip {
                json!({"amount": 1})
            } else {
                json!({})
            };
            assert_eq!(
                response,
                json!({"data": {"users": [{"id": "u", "txs": [transaction]}]}})
            );
            root.assert_async().await;
            entities.assert_async().await;
        }
    }

    #[ntex::test]
    async fn mutation_results_supply_context_before_the_next_root_field() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let step = Arc::new(AtomicUsize::new(0));
        let mut mocks = Vec::new();
        for (index, alias, currency) in [(0, "first", "USD"), (1, "second", "EUR")] {
            let root_step = Arc::clone(&step);
            mocks.push(
                server
                    .mock("POST", "/a")
                    .match_request(move |r| {
                        let request = body(r);
                        let query = request["query"].as_str().unwrap();
                        if !query.starts_with("mutation") || !query.contains(alias) {
                            return false;
                        }
                        assert_eq!(root_step.fetch_add(1, Ordering::SeqCst), index * 2);
                        true
                    })
                    .with_status(200)
                    .with_header("content-type", "application/json")
                    .with_body(
                        json!({
                            "data": {
                                alias: {
                                    "__typename": "User",
                                    "currency": currency,
                                    "txs": [{"__typename": "Transaction", "id": "same"}]
                                }
                            }
                        })
                        .to_string(),
                    )
                    .expect(1)
                    .create_async()
                    .await,
            );
            let entity_step = Arc::clone(&step);
            mocks.push(
                server
                    .mock("POST", "/a")
                    .match_request(move |r| {
                        let request = body(r);
                        let variable = format!("__context_{index}");
                        if request["variables"][&variable] != currency {
                            return false;
                        }
                        assert_eq!(
                            request["variables"]["representations"],
                            json!([{"__typename": "Transaction", "id": "same"}])
                        );
                        assert_eq!(entity_step.fetch_add(1, Ordering::SeqCst), index * 2 + 1);
                        true
                    })
                    .with_status(200)
                    .with_header("content-type", "application/json")
                    .with_body(json!({"data": {"_entities": [{"amount": index + 1}]}}).to_string())
                    .expect(1)
                    .create_async()
                    .await,
            );
        }
        let response = router
        .send_graphql_request(
            r#"mutation { first: updateUser(id: "1") { txs { amount } } second: updateUser(id: "2") { txs { amount } } }"#,
            None,
            None,
        )
        .await;
        let response: Value =
            serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
        assert_eq!(
            response,
            json!({"data": {"first": {"txs": [{"amount": 1}]}, "second": {"txs": [{"amount": 2}]}}})
        );
        assert_eq!(step.load(Ordering::SeqCst), 4);
        for mock in mocks {
            mock.assert_async().await;
        }
    }

    #[ntex::test]
    async fn batch_variables_do_not_overwrite_client_variables() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let root = server.mock("POST", "/a")
        .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"data":{
                "users":[
                    {"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]}
                ],
                "secondUsers":[
                    {"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]}
                ]
            }}"#,
        )
        .expect(1).create_async().await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let request = body(r);
                request["variables"]
                    == json!({
                        "__context_0": 7,
                        "__batch_reps_0": 9,
                        "__context_1": "USD",
                        "__context_2": "EUR",
                        "__batch_reps_1": [{"__typename": "Transaction", "id": "same"}],
                        "__batch_reps_2": [{"__typename": "Transaction", "id": "same"}]
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_e0":[{"amount":2}],"_e1":[{"amount":1}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let response = router
        .send_graphql_request(
            "query($__context_0: Int! = 7, $__batch_reps_0: Int! = 9) { users { txs { amount(multiplier: $__context_0) } } secondUsers { txs { amount(multiplier: $__batch_reps_0) } } }",
            None,
            None,
        )
        .await;
        let response: Value =
            serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
        assert_eq!(
            response,
            json!({"data": {"users": [{"txs": [{"amount": 1}]}], "secondUsers": [{"txs": [{"amount": 2}]}]}})
        );
        root.assert_async().await;
        entities.assert_async().await;
    }

    #[ntex::test]
    async fn non_null_contexts_from_exclusive_ancestors_are_fetched_separately() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-multi-type", &server).await;
        for (ancestor, entity, currency, variable) in [
            ("User", "Transaction", "USD", "__context_0"),
            ("Other", "Adjustment", "EUR", "__context_1"),
        ] {
            let root = server
                .mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    json!({
                        "data": {
                            "viewer": {
                                "__typename": ancestor,
                                "currency": currency,
                                "txs": [{"__typename": entity, "id": "same"}]
                            }
                        }
                    })
                    .to_string(),
                )
                .expect(1)
                .create_async()
                .await;
            let entities = server
                .mock("POST", "/a")
                .match_request(move |r| {
                    let request = body(r);
                    request["variables"]
                        == json!({
                            variable: currency,
                            "representations": [{"__typename": entity, "id": "same"}]
                        })
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_entities":[{"amount":1}]}}"#)
                .expect(1)
                .create_async()
                .await;
            let response = router
                .send_graphql_request(
                    "{ viewer { ... on User { txs { amount } } ... on Other { txs { amount } } } }",
                    None,
                    None,
                )
                .await;
            let response: Value =
                serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
            assert_eq!(
                response,
                json!({"data": {"viewer": {"txs": [{"amount": 1}]}}})
            );
            root.assert_async().await;
            entities.assert_async().await;
        }
    }

    #[ntex::test]
    async fn multi_type_fetch_reads_each_context_from_its_own_ancestor() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-outer", &server).await;
        let root = server
            .mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"outer":{
                "__typename":"Outer",
                "code":"outer",
                "user":{
                    "__typename":"User",
                    "currency":"USD",
                    "entries":[
                        {"__typename":"Transaction","id":"same"},
                        {"__typename":"Adjustment","id":"same"}
                    ]
                }
            }}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let entities = server
            .mock("POST", "/a")
            .match_request(|r| {
                let request = body(r);
                request["variables"]["__context_0"] == "USD"
                    && request["variables"]["__context_1"] == "outer"
                    && request["variables"]["representations"]
                        == json!([
                            {"__typename": "Transaction", "id": "same"},
                            {"__typename": "Adjustment", "id": "same"}
                        ])
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"amount":1},{"amount":2}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let response = router
        .send_graphql_request(
            "{ outer { user { entries { ... on Transaction { amount } ... on Adjustment { amount } } } } }",
            None,
            None,
        )
        .await;
        let response: Value =
            serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
        assert_eq!(
            response,
            json!({"data": {"outer": {"user": {"entries": [{"amount": 1}, {"amount": 2}]}}}})
        );
        root.assert_async().await;
        entities.assert_async().await;
    }
}
