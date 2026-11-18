use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [14] c              -
      [8] b               -
      [2] a               -
      [18] b              [14], [2]
      [10] a              [8]
      [13] c              [8]
      [19] c              [18]
      [17] a              [18]

    [14] c
      out
        { userInC { __typename id } }

    [8] b
      out
        { userInB { __typename id name } }

    [2] a
      out
        { userInA { __typename id } }

    [18] b   after [14], [2]
      alias _e0
        merge
          userInC
          userInA
        in
          ... on User { __typename id }
        out
          ... on User { name }

    [10] a   after [8]
      merge
        userInB
      in
        ... on User { __typename name id }
      out
        ... on User { aName }

    [13] c   after [8]
      merge
        userInB
      in
        ... on User { __typename name id }
      out
        ... on User { cName }

    [19] c   after [18]
      alias _e0
        merge
          userInC
          userInA
        in
          ... on User { __typename name id }
        out
          ... on User { cName }

    [17] a   after [18]
      alias _e0
        merge
          userInC
          userInA
        in
          ... on User { __typename name id }
        out
          ... on User { aName }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
                "operation": "{userInC{__typename id}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{userInB{__typename id name}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{userInA{__typename id}}"
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "BatchFetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on User{name}}}",
                "entityBatch": {
                  "aliases": [
                    {
                      "alias": "_e0",
                      "representationsVariableName": "__batch_reps_0",
                      "paths": [
                        [
                          {
                            "Field": "userInC"
                          }
                        ],
                        [
                          {
                            "Field": "userInA"
                          }
                        ]
                      ],
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
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "userInB"
                  }
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
                  {
                    "Field": "userInB"
                  }
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
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "BatchFetch",
                "serviceName": "c",
                "operationKind": "query",
                "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on User{cName}}}",
                "entityBatch": {
                  "aliases": [
                    {
                      "alias": "_e0",
                      "representationsVariableName": "__batch_reps_0",
                      "paths": [
                        [
                          {
                            "Field": "userInC"
                          }
                        ],
                        [
                          {
                            "Field": "userInA"
                          }
                        ]
                      ],
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
                  ]
                }
              },
              {
                "kind": "BatchFetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on User{aName}}}",
                "entityBatch": {
                  "aliases": [
                    {
                      "alias": "_e0",
                      "representationsVariableName": "__batch_reps_0",
                      "paths": [
                        [
                          {
                            "Field": "userInC"
                          }
                        ],
                        [
                          {
                            "Field": "userInA"
                          }
                        ]
                      ],
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] c               -
      [4] b               [2]
      [3] c               [4]

    [2] c
      out
        { userInC { __typename id } }

    [4] b   after [2]
      merge
        userInC
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [3] c   after [4]
      merge
        userInC
      in
        ... on User { __typename name id }
      out
        ... on User { cName }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "{userInC{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "userInC"
              }
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
              {
                "Field": "userInC"
              }
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [4] c               [5]

    [2] a
      out
        { userInA { __typename id } }

    [5] b   after [2]
      merge
        userInA
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [4] c   after [5]
      merge
        userInA
      in
        ... on User { __typename name id }
      out
        ... on User { cName }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{userInA{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "userInA"
              }
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
              {
                "Field": "userInA"
              }
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]
      [3] a               [4]

    [2] a
      out
        { userInA { __typename id } }

    [4] b   after [2]
      merge
        userInA
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [3] a   after [4]
      merge
        userInA
      in
        ... on User { __typename name id }
      out
        ... on User { aName }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{userInA{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "userInA"
              }
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
              {
                "Field": "userInA"
              }
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
