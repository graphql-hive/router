#[cfg(test)]
mod introspection_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter};

    #[ntex::test]
    async fn should_have_deprecated_input_values_in_introspection() {
        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
            ))
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request(
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
                None,
            )
            .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(resp.json_body_string_pretty().await, @r#"
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
        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
            ))
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request(
                r#"
            query IncludeOneOfInInputValues {
                TestInput: __type(name: "TestInput") {
                    isOneOf
                }
            }
        "#,
                None,
                None,
            )
            .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(resp.json_body_string_pretty().await, @r#"
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
        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
            ))
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request(
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
                None,
            )
            .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(resp.json_body_string_pretty().await, @r#"
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
    #[ntex::test]
    async fn should_have_specified_by_url_in_scalar_types() {
        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"supergraph:
                source: file
                path: "./supergraph-introspection-extended.graphql"
          "#,
            ))
            .build()
            .start()
            .await;

        let resp = router
            .send_graphql_request(
                r#"
            query IncludeOneOfInInputValues {
                MyScalar: __type(name: "MyScalar") {
                    specifiedByURL
                }
            }
        "#,
                None,
                None,
            )
            .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(resp.json_body_string_pretty().await, @r#"
        {
          "data": {
            "MyScalar": {
              "specifiedByURL": "https://example.com/my-scalar-spec"
            }
          }
        }
        "#);
    }
}
