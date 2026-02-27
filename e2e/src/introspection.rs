#[cfg(test)]
mod introspection_e2e_tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness,
    };
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    #[ntex::test]
    async fn should_have_deprecated_input_values_in_introsection() {
        let app = init_router_from_config_inline(
            r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
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
                            deprecationReason
                        }
                    }
                }
                TestInput: __type(name: "TestInput") {
                    inputFields(includeDeprecated: true) {
                        name
                        isDeprecated
                        deprecationReason
                    }
                }
                __schema {
                    directives {
                        name
                        args {
                            name
                            isDeprecated
                            deprecationReason
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

        insta::assert_snapshot!(to_string_pretty(&json_body).expect("Failed to serialize JSON for snapshot"), @r#"
        {
          "data": {
            "Query": {
              "fields": [
                {
                  "name": "testField",
                  "args": [
                    {
                      "name": "oldArg",
                      "isDeprecated": true,
                      "deprecationReason": "Use `newArg` instead"
                    },
                    {
                      "name": "newArg",
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ]
                }
              ]
            },
            "TestInput": {
              "inputFields": [
                {
                  "name": "oldField",
                  "isDeprecated": true,
                  "deprecationReason": "Use `newField` instead"
                },
                {
                  "name": "newField",
                  "isDeprecated": false,
                  "deprecationReason": null
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
                      "isDeprecated": true,
                      "deprecationReason": "Use `newArg` instead"
                    },
                    {
                      "name": "newArg",
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ]
                },
                {
                  "name": "skip",
                  "args": [
                    {
                      "name": "if",
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ]
                },
                {
                  "name": "include",
                  "args": [
                    {
                      "name": "if",
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ]
                },
                {
                  "name": "deprecated",
                  "args": [
                    {
                      "name": "reason",
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ]
                },
                {
                  "name": "specifiedBy",
                  "args": [
                    {
                      "name": "url",
                      "isDeprecated": false,
                      "deprecationReason": null
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
    #[ntex::test]
    async fn should_have_is_one_of_in_input_values() {
        let app = init_router_from_config_inline(
            r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query IncludeOneOfInInputValues {
                TestInput: __type(name: "TestInput") {
                    isOneOf
                }
            }
        "#,
            None,
        );
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).expect("Failed to deserialize JSON response");

        insta::assert_snapshot!(to_string_pretty(&json_body).expect("Failed to serialize JSON for snapshot"), @r#"
        {
          "data": {
            "TestInput": {
              "isOneOf": true
            }
          }
        }
        "#);
    }
    #[ntex::test]
    async fn should_have_default_values_in_input_values() {
        let app = init_router_from_config_inline(
            r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request(
            r#"
            query IncludeOneOfInInputValues {
                TestInput: __type(name: "TestInput") {
                    inputFields {
                        name
                        defaultValue
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

        insta::assert_snapshot!(to_string_pretty(&json_body).expect("Failed to serialize JSON for snapshot"), @r#"
        {
          "data": {
            "TestInput": {
              "inputFields": [
                {
                  "name": "oldField",
                  "defaultValue": null
                },
                {
                  "name": "newField",
                  "defaultValue": "\"newFieldDefaultValue\""
                }
              ]
            }
          }
        }
        "#);
    }
}
