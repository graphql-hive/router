use crate::testkit::{ClientResponseExt, Started, TestRouter};
use serde_json::{json, Value};

fn body(request: &mockito::Request) -> Value {
    serde_json::from_slice(request.body().unwrap()).unwrap()
}

async fn start_router(fixture: &str, server: &mockito::Server) -> TestRouter<Started> {
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
            "#
        ))
        .build()
        .start()
        .await
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"},{"__typename":"Transaction","id":"same"}]},{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"},{"__typename":"Transaction","id":"same"}]}]}}"#)
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"},{"__typename":"Transaction","id":"same"}]},{"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"},{"__typename":"Transaction","id":"same"}]},{"__typename":"User","currency":null,"txs":[{"__typename":"Transaction","id":"same"},{"__typename":"Transaction","id":"same"}]}]}}"#)
        .expect(1)
        .create_async()
        .await;
    let usd = server
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
    let eur = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["variables"]["__context_0"] == "EUR"
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_entities":[{"amount":2}]}}"#)
        .expect(1)
        .create_async()
        .await;
    let null = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["query"].as_str().unwrap().contains("_entities")
                && body["variables"]["__context_0"].is_null()
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_entities":[{"amount":0}]}}"#)
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
    usd.assert_async().await;
    eur.assert_async().await;
    null.assert_async().await;
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
        .with_body(r#"{"data":{"users":[{"child":{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"1"}]}}]}}"#)
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","wrapper":{"txs":[{"__typename":"Transaction","id":"1"}]}}]}}"#)
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
        .with_body(r#"{"data":{"account":{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"1"}]}}}"#)
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
        .with_body(r#"{"data":{"viewer":{"__typename":"Other","currency":"EUR","txs":[{"__typename":"Transaction","id":"1"}]}}}"#)
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
    let router = start_router("context-basic", &server).await;
    let _root = server
        .mock("POST", "/a")
        .match_request(|r| !body(r)["query"].as_str().unwrap().contains("_entities"))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"outer":{"__typename":"Outer","code":"outer","user":{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"1"}]}}}}"#)
        .expect(1)
        .create_async()
        .await;
    let entities = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["variables"]["__context_0"] == "USD" && body["variables"]["__context_1"] == "outer"
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"1"}]}]}}"#)
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
            body["variables"]["__context_0"] == "USD"
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
        .send_graphql_request(r#"{ users { txs { required } } }"#, None, None)
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
        .with_body(r#"{"data":{"renamed":[{"__typename":"User","currency":"user-id","_internal_qp_alias_0":"USD","txs":[{"__typename":"Transaction","id":"1"}]}]}}"#)
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]},{"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]}],"secondUsers":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]}]}}"#)
        .expect(1)
        .create_async()
        .await;
    let usd = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["variables"]["__context_0"] == "USD"
                && body["variables"]["__batch_reps_0"]
                    == json!([{"__typename":"Transaction","id":"same"}])
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_e0":[{"amount":1}],"_e1":[{"amount":1}]}}"#)
        .expect(1)
        .create_async()
        .await;
    let eur = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["variables"]["__context_0"] == "EUR"
                && body["variables"]["__batch_reps_0"] == json!([])
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_e0":[],"_e1":[{"amount":2}]}}"#)
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
    usd.assert_async().await;
    eur.assert_async().await;
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
        .with_body(r#"{"data":{"users":[{"__typename":"User","currency":"USD","txs":[{"__typename":"Transaction","id":"same"}]},{"__typename":"User","currency":"EUR","txs":[{"__typename":"Transaction","id":"same"}]}],"secondUsers":[]}}"#)
        .expect(1)
        .create_async()
        .await;
    let usd = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["query"].as_str().unwrap().contains("_e0")
                && body["variables"]["__batch_reps_0"] == json!([])
                && body["variables"].get("__context_1") == Some(&Value::Null)
                && body["variables"]["__context_0"] == "USD"
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_e1":[{"amount":1}]}}"#)
        .expect(1)
        .create_async()
        .await;
    let eur = server
        .mock("POST", "/a")
        .match_request(|r| {
            let body = body(r);
            body["query"].as_str().unwrap().contains("_e0")
                && body["variables"]["__batch_reps_0"] == json!([])
                && body["variables"].get("__context_1") == Some(&Value::Null)
                && body["variables"]["__context_0"] == "EUR"
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":{"_e1":[{"amount":2}]}}"#)
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
    usd.assert_async().await;
    eur.assert_async().await;
}
