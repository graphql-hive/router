use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn override_with_requires_many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userInA {
            id
            name
            aName
            cName
          }
          userInB {
            id
            name
            aName
            cName
          }
          userInC {
            id
            name
            aName
            cName
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "c") {
            {
              userInC {
                __typename
                id
              }
            }
          },
          Fetch(service: "b") {
            {
              userInB {
                __typename
                id
                name
              }
            }
          },
          Fetch(service: "a") {
            {
              userInA {
                __typename
                id
              }
            }
          },
        },
        Parallel {
          Flatten(path: "userInC") {
            Fetch(service: "b") {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  name
                }
              }
            },
          },
          Flatten(path: "userInB") {
            Fetch(service: "c") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
          },
          Flatten(path: "userInB") {
            Fetch(service: "a") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
          Flatten(path: "userInA") {
            Fetch(service: "b") {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  name
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "userInC") {
            Fetch(service: "c") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
          },
          Flatten(path: "userInC") {
            Fetch(service: "a") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
          Flatten(path: "userInA") {
            Fetch(service: "c") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
          },
          Flatten(path: "userInA") {
            Fetch(service: "a") {
              {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Fetch",
                "serviceName": "c",
                "operationKind": "query",
                "operation": "query{userInC{__typename id}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "query{userInB{__typename id name}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "query{userInA{__typename id}}"
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInB"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInB"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_with_requires_cname_in_c() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userInC {
            cName
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          {
            userInC {
              __typename
              id
            }
          }
        },
        Flatten(path: "userInC") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInC") {
          Fetch(service: "c") {
            {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                cName
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "query{userInC{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInC"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              "userInC"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_with_requires_cname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userInA {
            cName
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            userInA {
              __typename
              id
            }
          }
        },
        Flatten(path: "userInA") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInA") {
          Fetch(service: "c") {
            {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                cName
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{userInA{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_with_requires_aname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userInA {
            aName
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            userInA {
              __typename
              id
            }
          }
        },
        Flatten(path: "userInA") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInA") {
          Fetch(service: "a") {
            {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                aName
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{userInA{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}
