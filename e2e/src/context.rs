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
    async fn remote_context_producers_finish_before_the_consumer() {
        for multi_hop in [false, true] {
            let mut server = mockito::Server::new_async().await;
            let router = start_router(
                if multi_hop {
                    "context-multi-hop"
                } else {
                    "context-remote-producer"
                },
                &server,
            )
            .await;
            let step = Arc::new(AtomicUsize::new(0));
            let root = server
                .mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    json!({"data": {"t": if multi_hop {
                        json!({"__typename": "T", "a": {"__typename": "A", "id": "a"},
                            "u": {"__typename": "U", "id": "u"}})
                    } else {
                        json!({"__typename": "T", "id": "t",
                            "u": {"__typename": "U", "id": "u"}})
                    }}})
                    .to_string(),
                )
                .expect(1)
                .create_async()
                .await;
            let producer_step = Arc::clone(&step);
            let producer = server
                .mock("POST", "/b")
                .match_request(move |r| {
                    let request = body(r);
                    let typename = if multi_hop { "A" } else { "T" };
                    let id = if multi_hop { "a" } else { "t" };
                    if request["variables"]["representations"]
                        != json!([{"__typename": typename, "id": id}])
                    {
                        return false;
                    }
                    assert_eq!(producer_step.fetch_add(1, Ordering::SeqCst), 0);
                    true
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    json!({"data": {"_entities": [if multi_hop {
                    json!({"b": {"__typename": "B", "id": "b"}})
                } else { json!({"prop": "remote"}) }]}})
                    .to_string(),
                )
                .expect(1)
                .create_async()
                .await;
            let nested_step = Arc::clone(&step);
            let nested = server
                .mock("POST", "/a")
                .match_request(move |r| {
                    if body(r)["variables"]["representations"]
                        != json!([{"__typename": "B", "id": "b"}])
                    {
                        return false;
                    }
                    assert_eq!(nested_step.fetch_add(1, Ordering::SeqCst), 1);
                    true
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_entities":[{"c":{"prop":"remote"}}]}}"#)
                .expect(usize::from(multi_hop))
                .create_async()
                .await;
            let consumer_step = Arc::clone(&step);
            let consumer = server
                .mock("POST", "/a")
                .match_request(move |r| {
                    if body(r)["variables"]
                        != json!({
                            "representations": [{"__typename": "U", "id": "u"}],
                            "__context_0": "remote"
                        })
                    {
                        return false;
                    }
                    assert_eq!(
                        consumer_step.fetch_add(1, Ordering::SeqCst),
                        if multi_hop { 2 } else { 1 }
                    );
                    true
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_entities":[{"field":42}]}}"#)
                .expect(1)
                .create_async()
                .await;
            let response = router
                .send_graphql_request("{ t { u { field } } }", None, None)
                .await;
            let response: Value =
                serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
            assert_eq!(response, json!({"data": {"t": {"u": {"field": 42}}}}));
            assert_eq!(step.load(Ordering::SeqCst), if multi_hop { 3 } else { 2 });
            root.assert_async().await;
            producer.assert_async().await;
            nested.assert_async().await;
            consumer.assert_async().await;
        }
    }

    #[ntex::test]
    async fn query_root_context_injects_a_hidden_sibling() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-query-root", &server).await;
        let root = server.mock("POST", "/a")
            .match_request(|r| {
                let request = body(r);
                let query = request["query"].as_str().unwrap();
                !query.contains("_entities") && query.contains("me{locale}")
            })
            .with_status(200).with_header("content-type", "application/json")
            .with_body(r#"{"data":{"__typename":"Query","me":{"locale":"en-GB"},"product":{"__typename":"Product","id":"1"}}}"#)
            .expect(1).create_async().await;
        let consumer = server
            .mock("POST", "/a")
            .match_request(|r| {
                body(r)["variables"]
                    == json!({
                        "representations": [{"__typename": "Product", "id": "1"}],
                        "__context_0": "en-GB"
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"price":42}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let response = router
            .send_graphql_request("{ product { price } }", None, None)
            .await;
        let response: Value =
            serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
        assert_eq!(response, json!({"data": {"product": {"price": 42}}}));
        root.assert_async().await;
        consumer.assert_async().await;
    }

    #[ntex::test]
    async fn list_context_argument_preserves_nullable_elements() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-list-argument", &server).await;
        let root = server.mock("POST", "/a")
            .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
            .with_status(200).with_header("content-type", "application/json")
            .with_body(r#"{"data":{"t":{"__typename":"T","prop":["USD",null,"EUR"],"u":{"__typename":"U","id":"1"}}}}"#)
            .expect(1).create_async().await;
        let consumer = server
            .mock("POST", "/a")
            .match_request(|r| {
                let request = body(r);
                request["variables"]
                    == json!({
                        "representations": [{"__typename": "U", "id": "1"}],
                        "__context_0": ["USD", null, "EUR"]
                    })
                    && request["query"]
                        .as_str()
                        .unwrap()
                        .contains("$__context_0:[String]")
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[{"field":3}]}}"#)
            .expect(1)
            .create_async()
            .await;
        let response = router
            .send_graphql_request("{ t { u { field } } }", None, None)
            .await;
        let response: Value =
            serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
        assert_eq!(response, json!({"data": {"t": {"u": {"field": 3}}}}));
        root.assert_async().await;
        consumer.assert_async().await;
    }

    #[ntex::test]
    async fn context_fetch_failures_preserve_dependencies_and_null_bubbling() {
        for producer_fails in [true, false] {
            let mut server = mockito::Server::new_async().await;
            let router = start_router_with_config(
                "context-fetch-failures",
                &server,
                r#"
            error_masking:
              all:
                enabled: false
            "#,
            )
            .await;
            let root_response = if producer_fails {
                json!({"data": null, "errors": [{"message": "producer failed", "path": ["t"]}]})
            } else {
                json!({"data": {"t": {"__typename": "T", "id": "t", "prop": "available",
                    "u": {"__typename": "U", "id": "u"}}}})
            };
            let root = server
                .mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(root_response.to_string())
                .expect(1)
                .create_async()
                .await;
            let consumer = server
                .mock("POST", "/a")
                .match_request(move |r| {
                    let request = body(r);
                    request["query"].as_str().unwrap().contains("_entities")
                        && (producer_fails
                            || request["variables"]
                                == json!({
                                    "representations": [{"__typename": "U", "id": "u"}],
                                    "__context_0": "available"
                                }))
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_entities":[{"field":42}]}}"#)
                .expect(usize::from(!producer_fails))
                .create_async()
                .await;
            let unrelated = server.mock("POST", "/b")
                .match_request(|r| body(r)["variables"]["representations"] == json!([{"__typename": "U", "id": "u"}]))
                .with_status(200).with_header("content-type", "application/json")
                .with_body(r#"{"data":null,"errors":[{"message":"unrelated failed","path":["_entities",0,"b"]}]}"#)
                .expect(usize::from(!producer_fails)).create_async().await;
            let query = if producer_fails {
                "{ t { id u { field } } }"
            } else {
                "{ t { id u { field b } } }"
            };
            let response = router.send_graphql_request(query, None, None).await;
            let response: Value =
                serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
            assert!(response["data"].is_null(), "{response}");
            if producer_fails {
                insta::assert_json_snapshot!(response, @r#"
                {
                  "data": null,
                  "errors": [
                    {
                      "message": "producer failed",
                      "path": [
                        "t"
                      ],
                      "extensions": {
                        "code": "DOWNSTREAM_SERVICE_ERROR",
                        "service": "a"
                      }
                    }
                  ]
                }
                "#);
            } else {
                insta::assert_json_snapshot!(response, @r#"
                {
                  "data": null,
                  "errors": [
                    {
                      "message": "unrelated failed",
                      "path": [
                        "_entities",
                        0,
                        "b"
                      ],
                      "extensions": {
                        "code": "DOWNSTREAM_SERVICE_ERROR",
                        "service": "b",
                        "affectedPath": "t.u"
                      }
                    }
                  ]
                }
                "#);
            }
            root.assert_async().await;
            consumer.assert_async().await;
            unrelated.assert_async().await;
        }
    }

    #[ntex::test]
    async fn shared_context_names_do_not_leak_between_unrelated_objects() {
        for conditions in [false, true] {
            let mut server = mockito::Server::new_async().await;
            let router = start_router(
                if conditions {
                    "context-shared-conditions"
                } else {
                    "context-shared-duck"
                },
                &server,
            )
            .await;
            let root = server.mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200).with_header("content-type", "application/json")
                .with_body(json!({"data": {
                    "foo": {"__typename": "Foo", "prop": "foo-value", "u": {"__typename": "U", "id": "same"}},
                    "bar": {"__typename": "Bar", if conditions { "prop2" } else { "prop" }: "bar-value", "u": {"__typename": "U", "id": "same"}}
                }}).to_string())
                .expect(1).create_async().await;
            let consumer = server
                .mock("POST", "/a")
                .match_request(|r| {
                    body(r)["variables"]
                        == json!({
                            "__batch_reps_0": [{"__typename": "U", "id": "same"}],
                            "__batch_reps_1": [{"__typename": "U", "id": "same"}],
                            "__context_0": "foo-value", "__context_1": "bar-value"
                        })
                })
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"data":{"_e0":[{"field":2}],"_e1":[{"field":1}]}}"#)
                .expect(1)
                .create_async()
                .await;
            let response = router
                .send_graphql_request("{ foo { u { field } } bar { u { field } } }", None, None)
                .await;
            let response: Value =
                serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
            assert_eq!(
                response,
                json!({"data": {"foo": {"u": {"field": 1}}, "bar": {"u": {"field": 2}}}})
            );
            root.assert_async().await;
            consumer.assert_async().await;
        }
    }

    #[ntex::test]
    async fn chunks_context_groups_into_aliased_requests() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router_with_config(
            "context-basic",
            &server,
            r#"
            traffic_shaping:
              all:
                max_aliases_per_request: 15
            "#,
        )
        .await;
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
    async fn sends_all_context_groups_in_one_request_by_default() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router("context-basic", &server).await;
        let users: Vec<_> = (0..40)
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
        // listed backwards, the order of the aliases in the response doesn't matter
        let aliased_data: serde_json::Map<_, _> = (0..40)
            .rev()
            .map(|index| (format!("_entities_{index}"), json!([{"amount": index}])))
            .collect();
        let aliased = server
            .mock("POST", "/a")
            .match_request(|r| {
                let body = body(r);
                body["query"].as_str().unwrap().contains("_entities_39:")
                    && (0..40).all(|index| {
                        body["variables"][format!("__context_0_{index}")] == index.to_string()
                    })
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"data": aliased_data}).to_string())
            .expect(1)
            .create_async()
            .await;

        let res = router
            .send_graphql_request("{ users { txs { amount } } }", None, None)
            .await;
        let response: Value = serde_json::from_str(&res.json_body_string_pretty().await).unwrap();
        let expected: Vec<_> = (0..40)
            .map(|index| json!({"txs": [{"amount": index}]}))
            .collect();
        assert_eq!(response, json!({"data": {"users": expected}}));
        root.assert_async().await;
        aliased.assert_async().await;
    }

    #[ntex::test]
    async fn max_aliases_per_request_is_configurable_per_subgraph() {
        let mut server = mockito::Server::new_async().await;
        let router = start_router_with_config(
            "context-basic",
            &server,
            r#"
            traffic_shaping:
              subgraphs:
                a:
                  max_aliases_per_request: 2
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
        for (typename, currency) in [("User", "USD"), ("Other", "EUR")] {
            let root = server
                .mock("POST", "/a")
                .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    json!({"data": {"account": {
                        "__typename": typename,
                        "currency": currency,
                        "txs": [{"__typename": "Transaction", "id": "1"}]
                    }}})
                    .to_string(),
                )
                .expect(1)
                .create_async()
                .await;
            let entities = server
                .mock("POST", "/a")
                .match_request(move |r| {
                    body(r)["variables"]
                        == json!({
                            "__context_0": currency,
                            "representations": [{"__typename": "Transaction", "id": "1"}]
                        })
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
            let response: Value =
                serde_json::from_str(&res.json_body_string_pretty().await).unwrap();
            assert_eq!(
                response,
                json!({"data": {"account": {"txs": [{"fromInterface": 1}]}}})
            );
            root.assert_async().await;
            entities.assert_async().await;
        }
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
        // a non-null contextual argument only composes with a `__typename` selection
        let router = start_router("context-non-null", &server).await;
        for (ancestor, entity, variable) in [
            ("User", "Transaction", "__context_0"),
            ("Other", "Adjustment", "__context_1"),
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
                            variable: ancestor,
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
