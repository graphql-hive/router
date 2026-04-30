#[cfg(test)]
mod introspection_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter};

    #[ntex::test]
    async fn should_work_correctly_for_repeatable_directives() {
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

                query IntrospectionQuery {
                  __schema {

                    queryType { name kind }
                    mutationType { name kind }
                    subscriptionType { name kind }
                    types {
                      ...FullType
                    }
                    directives {
                      name
                      description
                      isRepeatable
                      locations
                      args {
                        ...InputValue
                      }
                    }
                  }
                }

                fragment FullType on __Type {
                  kind
                  name
                  description


                  fields(includeDeprecated: true) {
                    name
                    description
                    args {
                      ...InputValue
                    }
                    type {
                      ...TypeRef
                    }
                    isDeprecated
                    deprecationReason
                  }
                  inputFields {
                    ...InputValue
                  }
                  interfaces {
                    ...TypeRef
                  }
                  enumValues(includeDeprecated: true) {
                    name
                    description
                    isDeprecated
                    deprecationReason
                  }
                  possibleTypes {
                    ...TypeRef
                  }
                }

                fragment InputValue on __InputValue {
                  name
                  description
                  type { ...TypeRef }
                  defaultValue


                }

                fragment TypeRef on __Type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                      ofType {
                        kind
                        name
                        ofType {
                          kind
                          name
                          ofType {
                            kind
                            name
                            ofType {
                              kind
                              name
                              ofType {
                                kind
                                name
                                ofType {
                                  kind
                                  name
                                  ofType {
                                    kind
                                    name
                                  }
                                }
                              }
                            }
                          }
                        }
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
        insta::assert_snapshot!(resp.json_body_string_pretty().await, @r###"
        {
          "data": {
            "__schema": {
              "queryType": {
                "name": "Query",
                "kind": "OBJECT"
              },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "__Field",
                  "description": null,
                  "fields": [
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "String",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "args",
                      "description": null,
                      "args": [
                        {
                          "name": "includeDeprecated",
                          "description": null,
                          "type": {
                            "kind": "SCALAR",
                            "name": "Boolean",
                            "ofType": null
                          },
                          "defaultValue": "false"
                        }
                      ],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "OBJECT",
                              "name": "__InputValue",
                              "ofType": null
                            }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "type",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "OBJECT",
                          "name": "__Type",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "isDeprecated",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "deprecationReason",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "ID",
                  "description": "The `ID` scalar type represents a unique identifier, often used to refetch an object or as key for a cache. The ID type appears in a JSON response as a String; however, it is not intended to be human-readable. When expected as an input type, any string (such as `\"4\"`) or integer (such as `4`) input value will be accepted as an ID.",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "INPUT_OBJECT",
                  "name": "TestInput",
                  "description": null,
                  "fields": null,
                  "inputFields": [
                    {
                      "name": "oldField",
                      "description": null,
                      "type": {
                        "kind": "SCALAR",
                        "name": "MyScalar",
                        "ofType": null
                      },
                      "defaultValue": null
                    },
                    {
                      "name": "newField",
                      "description": null,
                      "type": {
                        "kind": "SCALAR",
                        "name": "MyScalar",
                        "ofType": null
                      },
                      "defaultValue": "\"newFieldDefaultValue\""
                    }
                  ],
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "testField",
                      "description": null,
                      "args": [
                        {
                          "name": "oldArg",
                          "description": null,
                          "type": {
                            "kind": "INPUT_OBJECT",
                            "name": "TestInput",
                            "ofType": null
                          },
                          "defaultValue": null
                        },
                        {
                          "name": "newArg",
                          "description": null,
                          "type": {
                            "kind": "INPUT_OBJECT",
                            "name": "TestInput",
                            "ofType": null
                          },
                          "defaultValue": null
                        }
                      ],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "ENUM",
                  "name": "__DirectiveLocation",
                  "description": null,
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": [
                    {
                      "name": "QUERY",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "MUTATION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "SUBSCRIPTION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "FIELD",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "FRAGMENT_DEFINITION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "FRAGMENT_SPREAD",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INLINE_FRAGMENT",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "SCHEMA",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "SCALAR",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "OBJECT",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "FIELD_DEFINITION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "ARGUMENT_DEFINITION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INTERFACE",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "UNION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "ENUM",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "ENUM_VALUE",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INPUT_OBJECT",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INPUT_FIELD_DEFINITION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__Schema",
                  "description": null,
                  "fields": [
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "types",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "OBJECT",
                              "name": "__Type",
                              "ofType": null
                            }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "queryType",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "OBJECT",
                          "name": "__Type",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "mutationType",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "OBJECT",
                        "name": "__Type",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "subscriptionType",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "OBJECT",
                        "name": "__Type",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "directives",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "OBJECT",
                              "name": "__Directive",
                              "ofType": null
                            }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "String",
                  "description": "The `String` scalar type represents textual data, represented as UTF-8 character sequences. The String type is most often used by GraphQL to represent free-form human-readable text.",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "Float",
                  "description": "The `Float` scalar type represents signed double-precision fractional values as specified by [IEEE 754](https://en.wikipedia.org/wiki/IEEE_floating_point). Float can represent values between -(2^53 - 1) and 2^53 - 1, inclusive.",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__InputValue",
                  "description": null,
                  "fields": [
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "String",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "type",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "OBJECT",
                          "name": "__Type",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "defaultValue",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "isDeprecated",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "deprecationReason",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__EnumValue",
                  "description": null,
                  "fields": [
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "String",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "isDeprecated",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "deprecationReason",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__Type",
                  "description": null,
                  "fields": [
                    {
                      "name": "kind",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "ENUM",
                          "name": "__TypeKind",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "specifiedByURL",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "fields",
                      "description": null,
                      "args": [
                        {
                          "name": "includeDeprecated",
                          "description": null,
                          "type": {
                            "kind": "SCALAR",
                            "name": "Boolean",
                            "ofType": null
                          },
                          "defaultValue": "false"
                        }
                      ],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": {
                          "kind": "NON_NULL",
                          "name": null,
                          "ofType": {
                            "kind": "OBJECT",
                            "name": "__Field",
                            "ofType": null
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "interfaces",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": {
                          "kind": "NON_NULL",
                          "name": null,
                          "ofType": {
                            "kind": "OBJECT",
                            "name": "__Type",
                            "ofType": null
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "possibleTypes",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": {
                          "kind": "NON_NULL",
                          "name": null,
                          "ofType": {
                            "kind": "OBJECT",
                            "name": "__Type",
                            "ofType": null
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "enumValues",
                      "description": null,
                      "args": [
                        {
                          "name": "includeDeprecated",
                          "description": null,
                          "type": {
                            "kind": "SCALAR",
                            "name": "Boolean",
                            "ofType": null
                          },
                          "defaultValue": "false"
                        }
                      ],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": {
                          "kind": "NON_NULL",
                          "name": null,
                          "ofType": {
                            "kind": "OBJECT",
                            "name": "__EnumValue",
                            "ofType": null
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "inputFields",
                      "description": null,
                      "args": [
                        {
                          "name": "includeDeprecated",
                          "description": null,
                          "type": {
                            "kind": "SCALAR",
                            "name": "Boolean",
                            "ofType": null
                          },
                          "defaultValue": "false"
                        }
                      ],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": {
                          "kind": "NON_NULL",
                          "name": null,
                          "ofType": {
                            "kind": "OBJECT",
                            "name": "__InputValue",
                            "ofType": null
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "ofType",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "OBJECT",
                        "name": "__Type",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "isOneOf",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "Boolean",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "ENUM",
                  "name": "__TypeKind",
                  "description": null,
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": [
                    {
                      "name": "SCALAR",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "OBJECT",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INTERFACE",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "UNION",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "ENUM",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "INPUT_OBJECT",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "LIST",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "NON_NULL",
                      "description": null,
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__Directive",
                  "description": null,
                  "fields": [
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "String",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "description",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "locations",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "ENUM",
                              "name": "__DirectiveLocation",
                              "ofType": null
                            }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "args",
                      "description": null,
                      "args": [
                        {
                          "name": "includeDeprecated",
                          "description": null,
                          "type": {
                            "kind": "SCALAR",
                            "name": "Boolean",
                            "ofType": null
                          },
                          "defaultValue": "false"
                        }
                      ],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "OBJECT",
                              "name": "__InputValue",
                              "ofType": null
                            }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "isRepeatable",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "MyScalar",
                  "description": null,
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "Int",
                  "description": "The `Int` scalar type represents non-fractional signed whole numeric values. Int can represent values between -(2^31) and 2^31 - 1.",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "Boolean",
                  "description": "The `Boolean` scalar type represents `true` or `false` values.",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                }
              ],
              "directives": [
                {
                  "name": "test_directive",
                  "description": null,
                  "isRepeatable": false,
                  "locations": [
                    "FIELD_DEFINITION"
                  ],
                  "args": [
                    {
                      "name": "oldArg",
                      "description": null,
                      "type": {
                        "kind": "INPUT_OBJECT",
                        "name": "TestInput",
                        "ofType": null
                      },
                      "defaultValue": null
                    },
                    {
                      "name": "newArg",
                      "description": null,
                      "type": {
                        "kind": "INPUT_OBJECT",
                        "name": "TestInput",
                        "ofType": null
                      },
                      "defaultValue": null
                    }
                  ]
                },
                {
                  "name": "skip",
                  "description": "Directs the executor to skip this field or fragment when the `if` argument is true.",
                  "isRepeatable": false,
                  "locations": [
                    "FIELD",
                    "FRAGMENT_SPREAD",
                    "INLINE_FRAGMENT"
                  ],
                  "args": [
                    {
                      "name": "if",
                      "description": "Skipped when true.",
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "defaultValue": null
                    }
                  ]
                },
                {
                  "name": "include",
                  "description": "Directs the executor to include this field or fragment only when the `if` argument is true.",
                  "isRepeatable": false,
                  "locations": [
                    "FIELD",
                    "FRAGMENT_SPREAD",
                    "INLINE_FRAGMENT"
                  ],
                  "args": [
                    {
                      "name": "if",
                      "description": "Included when true.",
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "Boolean",
                          "ofType": null
                        }
                      },
                      "defaultValue": null
                    }
                  ]
                },
                {
                  "name": "deprecated",
                  "description": null,
                  "isRepeatable": false,
                  "locations": [
                    "FIELD_DEFINITION",
                    "ENUM_VALUE",
                    "INPUT_FIELD_DEFINITION"
                  ],
                  "args": [
                    {
                      "name": "reason",
                      "description": null,
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "defaultValue": "\"No longer supported\""
                    }
                  ]
                },
                {
                  "name": "specifiedBy",
                  "description": null,
                  "isRepeatable": false,
                  "locations": [
                    "SCALAR"
                  ],
                  "args": [
                    {
                      "name": "url",
                      "description": null,
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "String",
                          "ofType": null
                        }
                      },
                      "defaultValue": null
                    }
                  ]
                },
                {
                  "name": "oneOf",
                  "description": null,
                  "isRepeatable": false,
                  "locations": [
                    "OBJECT",
                    "INTERFACE",
                    "UNION"
                  ],
                  "args": []
                }
              ]
            }
          }
        }
        "###);
    }

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
