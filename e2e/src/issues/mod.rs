#[cfg(test)]
mod issues_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter};

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/880
    async fn issue_880_null_in_required_field() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.880.graphql
                  query_planner:
                    allow_expose: true
                  override_subgraph_urls:
                    accounts:
                      url: "http://{host}/accounts"
                    products:
                      url: "http://{host}/products"
                  "#
            ))
            .build()
            .start()
            .await;

        // QueryPlan {
        //   Sequence {
        //     Fetch(service: "products") {
        //       {
        //         ad(id: "1") {
        //           id
        //           branch {
        //             __typename
        //             id
        //           }
        //         }
        //       }
        //     },
        let products_query_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("ad(")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "ad": { "id": "1", "branch": { "__typename": "Branch", "id": "branch-1" } }
                  }
                }
                "#,
            )
            .create();

        // Flatten(path: "ad.branch") {
        //   Fetch(service: "accounts") {
        //     {
        //       ... on Branch {
        //         __typename
        //         id
        //       }
        //     } =>
        //     {
        //       ... on Branch {
        //         contactOptions {
        //           email
        //           user {
        //             name
        //             id
        //           }
        //         }
        //       }
        //     }
        //   },
        // },
        let accounts_mock = server
            .mock("POST", "/accounts")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [
                    { "__typename": "Branch", "id": "branch-1", "contactOptions": null }
                  ]
                }
              }
              "#,
            )
            .create();

        //     Flatten(path: "ad") {
        //       Fetch(service: "products") {
        //         {
        //           ... on Ad {
        //             __typename
        //             branch {
        //               contactOptions {
        //                 email
        //                 user {
        //                   id
        //                   name
        //                 }
        //               }
        //             }
        //             id
        //           }
        //         } =>
        //         {
        //           ... on Ad {
        //             contactOptions {
        //               email
        //             }
        //           }
        //         }
        //       },
        //     },
        //   },
        // },
        let _products_entities_mock_valid_json = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();
                if !body_str.contains("$representations") {
                    return false;
                }

                sonic_rs::from_slice::<sonic_rs::Value>(body).is_ok()
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [
                    { "__typename": "Ad", "contactOptions": null }
                  ]
                }
              }
              "#,
            )
            .create();
        let _products_entities_mock_invalid_json = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();
                if !body_str.contains("$representations") {
                    return false;
                }

                sonic_rs::from_slice::<sonic_rs::Value>(body).is_err()
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [null]
                },
                "errors": [
                  { "message": "invalid json" }
                ]
              }
              "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                "{ ad(id: \"1\") { id contactOptions { email } } }",
                None,
                None,
            )
            .await;

        accounts_mock.assert();
        products_query_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "ad": {
              "id": "1",
              "contactOptions": null
            }
          }
        }
        "#);
    }
}
