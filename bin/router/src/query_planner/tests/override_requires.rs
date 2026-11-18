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
      [20] c              -
      [11] b              -
      [2] a               -
      [25] b              [20], [2]
      [17] c              [11]
      [13] a              [11]
      [26] c              [25]
      [23] a              [25]

    [20] c
      out
        { userInC { __typename id } }

    [11] b
      out
        { userInB { __typename id name } }

    [2] a
      out
        { userInA { __typename id } }

    [25] b   after [20], [2]
      alias _e0
        merge
          userInC
          userInA
        in
          ... on User { __typename id }
        out
          ... on User { name }

    [17] c   after [11]
      merge
        userInB
      in
        ... on User { __typename name id }
      out
        ... on User { cName }

    [13] a   after [11]
      merge
        userInB
      in
        ... on User { __typename name id }
      out
        ... on User { aName }

    [26] c   after [25]
      alias _e0
        merge
          userInC
          userInA
        in
          ... on User { __typename name id }
        out
          ... on User { cName }

    [23] a   after [25]
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
      [5] b               [2]
      [3] c               [5]

    [2] c
      out
        { userInC { __typename id } }

    [5] b   after [2]
      merge
        userInC
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [3] c   after [5]
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
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        { userInA { __typename id } }

    [6] b   after [2]
      merge
        userInA
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [4] c   after [6]
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
      [5] b               [2]
      [3] a               [5]

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

    [3] a   after [5]
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
