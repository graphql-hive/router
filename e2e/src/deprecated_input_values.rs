#[cfg(test)]
mod deprecated_input_values_e2e_tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness,
    };
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    #[ntex::test]
    async fn should_have_deprecated_input_values_in_intropsection() {
        let app = init_router_from_config_inline(
            r#"supergraph:
                source: file
                path: "./src/supergraph-deprecated_input_values.graphql"
          "#,
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query IncludeDeprecatedInputValues {
                Query: __type(name: "Query") {
                    fields {
                        name
                        args(includeDeprecated: true) {
                            name
                            isDeprecated
                        }
                    }
                }
                TestInput: __type(name: "TestInput") {
                    inputFields(includeDeprecated: true) {
                        name
                        isDeprecated
                    }
                }
                __schema {
                    directives {
                        name
                        args {
                            name
                            isDeprecated
                        }
                    }
                }
            }
        "#,
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).expect("Failed to deserialize JSON response");

        insta::assert_snapshot!(to_string_pretty(&json_body).unwrap(), @r#"
        {
          "data": {
            "Query": {
              "fields": [
                {
                  "name": "testField",
                  "args": [
                    {
                      "name": "oldArg",
                      "isDeprecated": true
                    },
                    {
                      "name": "newArg",
                      "isDeprecated": false
                    }
                  ]
                }
              ]
            },
            "TestInput": {
              "inputFields": [
                {
                  "name": "oldField",
                  "isDeprecated": true
                },
                {
                  "name": "newField",
                  "isDeprecated": false
                }
              ]
            },
            "__schema": {
              "directives": [
                {
                  "name": "test_directive",
                  "args": [
                    {
                      "name": "oldArg",
                      "isDeprecated": true
                    },
                    {
                      "name": "newArg",
                      "isDeprecated": false
                    }
                  ]
                },
                {
                  "name": "skip",
                  "args": [
                    {
                      "name": "if",
                      "isDeprecated": false
                    }
                  ]
                },
                {
                  "name": "include",
                  "args": [
                    {
                      "name": "if",
                      "isDeprecated": false
                    }
                  ]
                },
                {
                  "name": "deprecated",
                  "args": [
                    {
                      "name": "reason",
                      "isDeprecated": false
                    }
                  ]
                },
                {
                  "name": "specifiedBy",
                  "args": [
                    {
                      "name": "url",
                      "isDeprecated": false
                    }
                  ]
                },
                {
                  "name": "oneOf",
                  "args": []
                }
              ]
            }
          }
        }
        "#);
    }
}
