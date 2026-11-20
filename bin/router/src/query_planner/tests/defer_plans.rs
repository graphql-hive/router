//! `@defer` operations over a set of composed supergraphs (`fixtures/defer_plans`). These
//! snapshot the display plan, to read through and spot a deferred fetch in the wrong place.

use std::error::Error;

use crate::query_planner::tests::testkit::{
    build_defer_display_plan, build_query_plan_with_defaults, init_logger,
};
use crate::query_planner::utils::parsing::parse_operation;

fn show(supergraph: &str, query: &str) -> Result<String, Box<dyn Error>> {
    init_logger();
    let path = format!("src/query_planner/tests/fixtures/defer_plans/{supergraph}.graphql");
    match build_defer_display_plan(&path, parse_operation(query)) {
        Ok(plan) => Ok(plan.to_string()),
        // `@defer(if: false)` is gone after normalization, so it's a plain plan
        Err(e) if e.to_string() == "the operation has no @defer" => {
            Ok(build_query_plan_with_defaults(&path, parse_operation(query))?.to_string())
        }
        Err(e) => Err(e),
    }
}

#[test]
fn same_subgraph_defer_under_a_value_type() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("same_subgraph_defer_under_a_value_type", r#"
            {
              t {
                v {
                  a
                  ... @defer {
                    b
                  }
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v { a } } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {t{__typename id}}
            },
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v{a b}}}}
              },
            },
          },
        }, [
          Deferred(depends: [], path: "t.v") {
            { b }:
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn same_subgraph_defer_before_a_field() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("same_subgraph_defer_before_a_field", r#"
            {
              t {
                ... @defer {
                  v1
                }
                v2
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v2 } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {t{__typename v1 id}}
            },
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
              },
            },
          },
        }, [
          Deferred(depends: [], path: "t") {
            { v1 }:
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_resuming_in_the_same_subgraph() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_resuming_in_the_same_subgraph", r#"
          {
            t {
              v0
              ... @defer {
                v1
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v0 } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{__typename v0 id}}
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { v1 }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_multiple_fields_in_different_subgraphs() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_multiple_fields_in_different_subgraphs", r#"
          {
            t {
              v0
              ... @defer {
                v1
                v2
                v3
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v0 } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{__typename v0 id}}
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { v1 v2 v3 }:
            Parallel {
              Flatten(path: "t") {
                Fetch(service: "Subgraph3", id: 5) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v3}}}
                },
              },
              Flatten(path: "t") {
                Fetch(service: "Subgraph2", id: 4) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
                },
              },
              Flatten(path: "t") {
                Fetch(service: "Subgraph1", id: 3) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn multiple_non_nested_defer_plus_label_handling() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("multiple_non_nested_defer_plus_label_handling", r#"
          {
            t {
              v0
              ... @defer(label: "defer_v1") {
                v1
              }
              ... @defer {
                v2
              }
              v3 {
                x
                ... @defer(label: "defer_in_v3") {
                  y
                }
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v0 v3 { x } } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {t{__typename v0 id}}
            },
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 5) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v3{__typename id}}}}
              },
            },
            Flatten(path: "t.v3") {
              Fetch(service: "Subgraph3", id: 6) {
                {
                  ... on U {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on U{x}}}
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "t", label: "defer_v1") {
            { v1 }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
              },
            },
          },
          Deferred(depends: [2], path: "t") {
            { v2 }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 4) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
              },
            },
          },
          Deferred(depends: [5], path: "t.v3", label: "defer_in_v3") {
            { y }:
            Flatten(path: "t.v3") {
              Fetch(service: "Subgraph3", id: 7) {
                {
                  ... on U {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on U{y}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_value_types() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_on_value_types", r#"
          {
            me {
              ... @defer {
                messages {
                  ... @defer {
                    body {
                      lines
                    }
                  }
                }
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "Subgraph1", id: 2) {
            {me{__typename id}}
          },
        }, [
          Deferred(depends: [2], path: "me") {
            Defer {
              Primary {
                Flatten(path: "me") {
                  Fetch(service: "Subgraph2", id: 3) {
                    {
                      ... on User {
                        __typename
                        id
                      }
                    } =>
                    query($representations:[_Any!]!){_entities(representations: $representations){...on User{messages{body{lines}}}}}
                  },
                },
              }, [
                Deferred(depends: [], path: "me.messages.@") {
                  { body { lines } }:
                },
              ]
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn direct_nesting_on_entity() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("direct_nesting_on_entity", r#"
          {
            me {
              name
              ... @defer {
                age
                ... @defer {
                  address
                }
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { me { name } }:
          Fetch(service: "Subgraph1", id: 2) {
            {me{__typename name id}}
          },
        }, [
          Deferred(depends: [2], path: "me") {
            Defer {
              Primary {
                { age }:
                Flatten(path: "me") {
                  Fetch(service: "Subgraph2", id: 3) {
                    {
                      ... on User {
                        __typename
                        id
                      }
                    } =>
                    query($representations:[_Any!]!){_entities(representations: $representations){...on User{age}}}
                  },
                },
              }, [
                Deferred(depends: [2], path: "me") {
                  { address }:
                  Flatten(path: "me") {
                    Fetch(service: "Subgraph2", id: 4) {
                      {
                        ... on User {
                          __typename
                          id
                        }
                      } =>
                      query($representations:[_Any!]!){_entities(representations: $representations){...on User{address}}}
                    },
                  },
                },
              ]
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_entity_with_an_unused_key() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_on_entity_with_an_unused_key", r#"
            {
              t {
                ... @defer {
                  a
                  ... @defer {
                    b
                  }
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "Subgraph1", id: 2) {
            {t{a b}}
          },
        }, [
          Deferred(depends: [], path: "t") {
            Defer {
              Primary {
                { a }:
              }, [
                Deferred(depends: [], path: "t") {
                  { b }:
                },
              ]
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_mutation_in_same_subgraph() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_on_mutation_in_same_subgraph", r#"
            mutation mut {
              update1 {
                v0
                ... @defer {
                  v1
                }
              }
              update2 {
                v1
                ... @defer {
                  v0
                  v2
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { update1 { v0 } update2 { v1 } }:
          Fetch(service: "Subgraph1", id: 2) {
            mutation{update1{__typename v0 id} update2{__typename v1 id}}
          },
        }, [
          Deferred(depends: [2], path: "update1") {
            { v1 }:
            Flatten(path: "update1") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
              },
            },
          },
          Deferred(depends: [2], path: "update2") {
            { v0 v2 }:
            Parallel {
              Flatten(path: "update2") {
                Fetch(service: "Subgraph1", id: 5) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v0}}}
                },
              },
              Flatten(path: "update2") {
                Fetch(service: "Subgraph2", id: 6) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_mutation_on_different_subgraphs() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_on_mutation_on_different_subgraphs", r#"
            mutation mut {
              update1 {
                v0
                ... @defer {
                  v1
                }
              }
              update2 {
                v1
                ... @defer {
                  v0
                  v2
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { update1 { v0 } update2 { v1 } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              mutation{update1{__typename v0 id}}
            },
            Fetch(service: "Subgraph2", id: 4) {
              mutation{update2{__typename id}}
            },
            Flatten(path: "update2") {
              Fetch(service: "Subgraph1", id: 5) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "update1") {
            { v1 }:
            Flatten(path: "update1") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v1}}}
              },
            },
          },
          Deferred(depends: [4], path: "update2") {
            { v0 v2 }:
            Parallel {
              Flatten(path: "update2") {
                Fetch(service: "Subgraph2", id: 7) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
                },
              },
              Flatten(path: "update2") {
                Fetch(service: "Subgraph1", id: 6) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v0}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_multi_dependency_deferred_section() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_on_multi_dependency_deferred_section", r#"
          {
            t {
              v1
              v2
              v3
              ... @defer {
                v4
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v1 v2 v3 } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {t{__typename v1 id0}}
            },
            Parallel {
              Flatten(path: "t") {
                Fetch(service: "Subgraph3", id: 4) {
                  {
                    ... on T {
                      __typename
                      id0
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v3}}}
                },
              },
              Flatten(path: "t") {
                Fetch(service: "Subgraph2", id: 3) {
                  {
                    ... on T {
                      __typename
                      id0
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
                },
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { v4 }:
            Sequence {
              Parallel {
                Flatten(path: "t") {
                  Fetch(service: "Subgraph3", id: 7) {
                    {
                      ... on T {
                        __typename
                        id0
                      }
                    } =>
                    query($representations:[_Any!]!){_entities(representations: $representations){...on T{id2}}}
                  },
                },
                Flatten(path: "t") {
                  Fetch(service: "Subgraph2", id: 6) {
                    {
                      ... on T {
                        __typename
                        id0
                      }
                    } =>
                    query($representations:[_Any!]!){_entities(representations: $representations){...on T{id1}}}
                  },
                },
              },
              Flatten(path: "t") {
                Fetch(service: "Subgraph4", id: 5) {
                  {
                    ... on T {
                      __typename
                      id1
                      id2
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v4}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn requirements_of_deferred_fields_are_deferred() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("requirements_of_deferred_fields_are_deferred", r#"
            {
              t {
                v1
                ... @defer {
                  v2
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v1 } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{__typename v1 id}}
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { v2 }:
            Sequence {
              Flatten(path: "t") {
                Fetch(service: "Subgraph3", id: 6) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v3}}}
                },
              },
              Flatten(path: "t") {
                Fetch(service: "Subgraph2", id: 4) {
                  {
                    ... on T {
                      __typename
                      v3
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn provides_are_ignored_for_deferred_fields() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("provides_are_ignored_for_deferred_fields", r#"
            {
              t {
                v1
                ... @defer {
                  v2
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v1 } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{v1 v2}}
          },
        }, [
          Deferred(depends: [], path: "t") {
            { v2 }:
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_with_multiple_conditions_and_labels() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_with_multiple_conditions_and_labels", r#"
            query ($cond1: Boolean, $cond2: Boolean) {
              t {
                x
                ... @defer(if: $cond1, label: "foo") {
                  y
                }
                ... @defer(if: $cond2, label: "bar") {
                  u {
                    a
                    ... @defer(if: $cond1) {
                      b
                    }
                  }
                }
              }
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { x ... on T @skip(if: $cond1) { y } ... on T @skip(if: $cond2) { u { a ... on U @skip(if: $cond1) { b } } } } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              query($_d0:Boolean!, $_d1:Boolean!){t{x ...on T @skip(if: $_d0){...a} ...on T @include(if: $_d0){...a} ...on T @skip(if: $_d1){u{a ...on U @skip(if: $_d0){...b} ...on U @include(if: $_d0){...b}}} ...on T @include(if: $_d1){...a}}}

    fragment a on T {__typename id}
    fragment b on U {__typename id}

            },
            Parallel {
              Flatten(path: "t|[T].u|[U]") {
                Fetch(service: "Subgraph3", id: 5, if: !$_d0 && !$_d1) {
                  {
                    ... on U {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on U{b}}}
                },
              },
              Flatten(path: "t|[T]") {
                Fetch(service: "Subgraph2", id: 3, if: !$_d0) {
                  {
                    ... on T {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on T{y}}}
                },
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "t", label: "foo") {
            { ... on T @include(if: $cond1) { y } }:
            Flatten(path: "t|[T]") {
              Fetch(service: "Subgraph2", id: 4, if: $_d0) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{y}}}
              },
            },
          },
          Deferred(depends: [2], path: "t.u") {
            { ... on U @include(if: $cond1) { b } }:
            Flatten(path: "t|[T].u|[U]") {
              Fetch(service: "Subgraph3", id: 6, if: $_d0 && !$_d1) {
                {
                  ... on U {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on U{b}}}
              },
            },
          },
          Deferred(depends: [2], path: "t", label: "bar") {
            Defer {
              Primary {
                { ... on T @include(if: $cond2) { u { a ... on U @skip(if: $cond1) { b } } } }:
                Sequence {
                  Flatten(path: "t|[T]") {
                    Fetch(service: "Subgraph1", id: 7, if: $_d1) {
                      {
                        ... on T {
                          __typename
                          id
                        }
                      } =>
                      query($representations:[_Any!]!, $_d0:Boolean!){_entities(representations: $representations){...on T{u{a ...on U @skip(if: $_d0){...a} ...on U @include(if: $_d0){...a}}}}}

    fragment a on U {__typename id}

                    },
                  },
                  Flatten(path: "t|[T].u|[U]") {
                    Fetch(service: "Subgraph3", id: 8, if: $_d1 && !$_d0) {
                      {
                        ... on U {
                          __typename
                          id
                        }
                      } =>
                      query($representations:[_Any!]!){_entities(representations: $representations){...on U{b}}}
                    },
                  },
                },
              }, [
                Deferred(depends: [7], path: "t.u") {
                  { ... on U @include(if: $cond1) { b } }:
                  Flatten(path: "t|[T].u|[U]") {
                    Fetch(service: "Subgraph3", id: 9, if: $_d0 && $_d1) {
                      {
                        ... on U {
                          __typename
                          id
                        }
                      } =>
                      query($representations:[_Any!]!){_entities(representations: $representations){...on U{b}}}
                    },
                  },
                },
              ]
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn interface_has_different_definitions_between_subgraphs() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("interface_has_different_definitions_between_subgraphs", r#"
          query Dimensions {
            i {
              a
              b
              ... @defer {
                c
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { i { ... on T { a b } } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {i{__typename ...on T{__typename a id}}}
            },
            Flatten(path: "i|[T]") {
              Fetch(service: "Subgraph2", id: 4) {
                {
                  ... on T {
                    __typename
                    a
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{b}}}
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "i") {
            { ... on T { c } }:
            Flatten(path: "i|[T]") {
              Fetch(service: "Subgraph1", id: 7) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{c}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn fragments_expand_into_same_field_regardless_of_defer() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("fragments_expand_into_same_field_regardless_of_defer", r#"
            {
              t {
                ...Fragment1
                ...Fragment2 @defer
              }
            }

            fragment Fragment1 on T {
              x
              y
            }

            fragment Fragment2 on T {
              y
              z
            }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { x y } }:
          Sequence {
            Fetch(service: "Subgraph1", id: 2) {
              {t{__typename id}}
            },
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{x y}}}
              },
            },
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { y z }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 4) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{y z}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn do_not_merge_query_branches_with_defer() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("do_not_merge_query_branches_with_defer", r#"
          {
            t {
              a
              ... @defer {
                b
              }
              ... @defer {
                c
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { a } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{__typename a id}}
          },
        }, [
          Deferred(depends: [2], path: "t") {
            { b }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{b}}}
              },
            },
          },
          Deferred(depends: [2], path: "t") {
            { c }:
            Flatten(path: "t") {
              Fetch(service: "Subgraph2", id: 4) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{c}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_only_the_key_of_an_entity() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("defer_only_the_key_of_an_entity", r#"
          {
            t {
              v0
              ... @defer {
                id
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { t { v0 } }:
          Fetch(service: "Subgraph1", id: 2) {
            {t{__typename v0 id}}
          },
        }, [
          Deferred(depends: [], path: "t") {
            { id }:
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn the_path_in_defer_includes_traversed_fragments() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("the_path_in_defer_includes_traversed_fragments", r#"
          {
            i {
              ... on A {
                t {
                  v1
                  ... @defer {
                    v2
                  }
                }
              }
            }
          }

"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { i { ... on A { t { v1 } } } }:
          Fetch(service: "Subgraph1", id: 2) {
            {i{__typename ...on A{t{__typename v1 id}}}}
          },
        }, [
          Deferred(depends: [2], path: "i.t") {
            { v2 }:
            Flatten(path: "i|[A].t") {
              Fetch(service: "Subgraph1", id: 3) {
                {
                  ... on T {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on T{v2}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn sibling_defers_on_an_interface_from_the_same_subgraph() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          {
            node(id: "b1") {
              ... @defer { title }
              ... @defer { title }
            }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "catalog", id: 2) {
            {node(id: "b1"){__typename id}}
          },
        }, [
          Deferred(depends: [2], path: "node") {
            { ... on Book { title } ... on Movie { title } }:
            Flatten(path: "node") {
              Fetch(service: "catalog", id: 4) {
                {
                  ... on Book {
                    __typename
                    id
                  }
                  ... on Movie {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Book{title} ...on Movie{title}}}
              },
            },
          },
          Deferred(depends: [2], path: "node") {
            { ... on Book { title } ... on Movie { title } }:
            Flatten(path: "node") {
              Fetch(service: "catalog", id: 6) {
                {
                  ... on Book {
                    __typename
                    id
                  }
                  ... on Movie {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Book{title} ...on Movie{title}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn defer_on_an_interface_from_the_same_subgraph() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          {
            node(id: "b1") {
              ... @defer { title }
            }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "catalog", id: 2) {
            {node(id: "b1"){__typename id}}
          },
        }, [
          Deferred(depends: [2], path: "node") {
            { ... on Book { title } ... on Movie { title } }:
            Flatten(path: "node") {
              Fetch(service: "catalog", id: 4) {
                {
                  ... on Book {
                    __typename
                    id
                  }
                  ... on Movie {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Book{title} ...on Movie{title}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn sibling_defers_both_reading_from_a_subgraph_with_a_conditional_field(
) -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          query ($v: Boolean!) {
            shelf {
              items {
                ... on Book @defer { reviews { by @include(if: $v) { name } } }
                ... on Book @defer { rating }
              }
            }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "catalog", id: 2) {
            {shelf{items{__typename ...on Book{__typename id}}}}
          },
        }, [
          Deferred(depends: [2], path: "shelf.items.@") {
            { ... on Book { reviews { by @include(if: $v) { name } } } }:
            Sequence {
              Flatten(path: "shelf.items.@|[Book]") {
                Fetch(service: "reviews", id: 4) {
                  {
                    ... on Book {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!, $v:Boolean!){_entities(representations: $representations){...on Book{reviews{by @include(if: $v){__typename id}}}}}
                },
              },
              Flatten(path: "shelf.items.@|[Book].reviews.@.by") {
                Fetch(service: "catalog", id: 5, if: $v) {
                  {
                    ... on Person {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Person{name}}}
                },
              },
            },
          },
          Deferred(depends: [2], path: "shelf.items.@") {
            { ... on Book { rating } }:
            Flatten(path: "shelf.items.@|[Book]") {
              Fetch(service: "reviews", id: 3) {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Book{rating}}}
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn sibling_defers_with_the_same_field_one_under_a_skip() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          query ($v: Boolean!) {
            nodes {
              ... on Book @defer { summary reviews { by { email } } }
              ... on Book @defer { summary reviews { by { email @skip(if: $v) } } }
            }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "catalog", id: 2) {
            {nodes{__typename ...on Book{__typename id}}}
          },
        }, [
          Deferred(depends: [2], path: "nodes.@") {
            { ... on Book { summary reviews { by { email } } } }:
            Sequence {
              Flatten(path: "nodes.@|[Book]") {
                Fetch(service: "catalog", id: 6) {
                  {
                    ... on Book {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Book{title}}}
                },
              },
              Flatten(path: "nodes.@|[Book]") {
                Fetch(service: "reviews", id: 3) {
                  {
                    ... on Book {
                      __typename
                      id
                      title
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Book{reviews{by{__typename id}} summary}}}
                },
              },
              Flatten(path: "nodes.@|[Book].reviews.@.by") {
                Fetch(service: "people", id: 7) {
                  {
                    ... on Person {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Person{email}}}
                },
              },
            },
          },
          Deferred(depends: [2], path: "nodes.@") {
            { ... on Book { summary reviews { by { email @skip(if: $v) } } } }:
            Sequence {
              Flatten(path: "nodes.@|[Book]") {
                Fetch(service: "catalog", id: 11) {
                  {
                    ... on Book {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Book{title}}}
                },
              },
              Flatten(path: "nodes.@|[Book]") {
                Fetch(service: "reviews", id: 8) {
                  {
                    ... on Book {
                      __typename
                      id
                      title
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Book{reviews{by{__typename id}} summary}}}
                },
              },
              Flatten(path: "nodes.@|[Book].reviews.@.by") {
                Fetch(service: "people", id: 12, if: !$v) {
                  {
                    ... on Person {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Person{email}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn entity_call_of_the_primary_part_and_a_deferred_root_field() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          {
            nodes { ... on Movie { rating } id }
            ... @defer { nodes { ... on Movie { rating } } }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { nodes { ... on Movie { rating } id } }:
          Sequence {
            Fetch(service: "catalog", id: 2) {
              {nodes{__typename ...on Movie{__typename id} id}}
            },
            Flatten(path: "nodes.@|[Movie]") {
              Fetch(service: "reviews", id: 3) {
                {
                  ... on Movie {
                    __typename
                    id
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Movie{rating}}}
              },
            },
          },
        }, [
          Deferred(depends: [], path: "") {
            { nodes { ... on Movie { rating } } }:
            Sequence {
              Fetch(service: "catalog", id: 4) {
                {nodes{__typename ...on Movie{__typename id}}}
              },
              Flatten(path: "nodes.@|[Movie]") {
                Fetch(service: "reviews", id: 5) {
                  {
                    ... on Movie {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Movie{rating}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}

#[test]
fn key_field_also_asked_for_under_a_skip() -> Result<(), Box<dyn Error>> {
    insta::assert_snapshot!(show("catalog_reviews_people", r#"
          query ($v: Boolean!) {
            book(id: "b2") {
              ... @defer { id @skip(if: $v) summary __typename }
            }
          }
"#)?, @r#"
    QueryPlan {
      Defer {
        Primary {
          Fetch(service: "catalog", id: 2) {
            {book(id: "b2"){__typename id}}
          },
        }, [
          Deferred(depends: [2], path: "book") {
            { id @skip(if: $v) summary __typename }:
            Sequence {
              Flatten(path: "book") {
                Fetch(service: "catalog", id: 7) {
                  {
                    ... on Book {
                      __typename
                      id
                    }
                  } =>
                  query($representations:[_Any!]!, $v:Boolean!){_entities(representations: $representations){...on Book{__typename title id @skip(if: $v)}}}
                },
              },
              Flatten(path: "book") {
                Fetch(service: "reviews", id: 5) {
                  {
                    ... on Book {
                      __typename
                      title
                      id
                    }
                  } =>
                  query($representations:[_Any!]!){_entities(representations: $representations){...on Book{summary}}}
                },
              },
            },
          },
        ]
      },
    },
    "#);
    Ok(())
}
