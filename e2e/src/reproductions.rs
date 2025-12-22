#[cfg(test)]
mod reproductions_e2e_tests {
    use mockito::ServerOpts;
    use ntex::web::test;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness,
    };

    // Reproduction test for ROUTER-235
    #[ntex::test]
    async fn reprod_router_235() {
        let mut server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 4001,
            ..Default::default()
        })
        .await;

        let mock = server
            .mock("POST", "/graphql")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                    "content": [
                        {
                            "__typename": "ContentA",
                            "inner": null
                        },
                        {
                            "__typename": "ContentB",
                            "inner": [
                                {
                                    "__typename": "ContentInner",
                                    "id": "contentInner1"
                                }
                            ]
                        }
                    ]
                }
            }"#,
            )
            .create();

        let test_app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: file
              path: "{}/supergraph-router-235.graphql"
        "#,
            env!("CARGO_MANIFEST_DIR"),
        ))
        .await
        .expect("failed to start router");
        wait_for_readiness(&test_app.app).await;

        let resp = test::call_service(
            &test_app.app,
            init_graphql_request(
                r#"
                    query {
                        content {
                            ...ContentA
                            ...ContentB
                        }
                    }

                    fragment ContentA on ContentA {
                        inner {
                            id
                        }
                    }

                    fragment ContentB on ContentB {
                        inner {
                            id
                        }
                    }
                "#,
                None,
            )
            .to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body).unwrap();

        let expected = sonic_rs::json!({
            "data": {
                "content": [
                    {
                        "inner": null
                    },
                    {
                        "inner": [
                            {
                                "id": "contentInner1"
                            }
                        ]
                    }
                ]
            }
        })
        .to_string();

        assert_eq!(body_str, expected);

        mock.assert();
    }
}
