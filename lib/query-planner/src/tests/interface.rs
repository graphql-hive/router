use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Tests querying the `node` interface field using two different aliases (`account` and `chat`).
/// Verifies that aliases work correctly when querying the same interface field with different IDs.
#[test]
fn node_query_with_aliases_on_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            __typename
          }
          chat: node(id: "c1") {
            __typename
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          account: node(id: "a1") {
    ...a      }
          chat: node(id: "c1") {
    ...a      }
        }
        fragment a on Node {
          __typename
        }
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with inline fragments for concrete object types (`Account` and `Chat`).
/// Verifies that inline fragments on specific object types are handled correctly when returned from an interface field.
#[test]
fn node_query_with_inline_fragments_on_object_types() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            ... on Account {
              id
              username
            }
          }
          chat: node(id: "c1") {
            ... on Chat {
              id
              text
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "b") {
          {
            chat: node(id: "c1") {
              __typename
              ... on Chat {
                id
                text
              }
            }
          }
        },
        Fetch(service: "a") {
          {
            account: node(id: "a1") {
              __typename
              ... on Account {
                id
                username
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with inline fragments for types that do not match the actual type of the returned object.
/// Verifies that fragments for non-matching types do not cause errors and are handled gracefully.
#[test]
fn node_query_with_cross_type_inline_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "a") {
          {
            chat: node(id: "c1") {
              __typename
              ... on Account {
                id
              }
            }
          }
        },
        Fetch(service: "b") {
          {
            account: node(id: "a1") {
              __typename
              ... on Chat {
                id
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with both `__typename` and cross-type inline fragments.
/// Verifies that `__typename` is always available and that fragments for non-matching types are handled correctly.
#[test]
fn node_query_with_typename_and_cross_type_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            __typename
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            __typename
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "a") {
          {
            chat: node(id: "c1") {
              __typename
              ... on Account {
                id
              }
            }
          }
        },
        Fetch(service: "b") {
          {
            account: node(id: "a1") {
              __typename
              ... on Chat {
                id
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `chat` object field directly by ID.
/// Verifies that direct object type queries (not through an interface) are resolved as expected.
#[test]
fn direct_object_query_chat_by_id() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          chat(id: "c1") {
            id
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          chat(id: "c1") {
            id
          }
        }
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `account` object field directly by ID.
/// Verifies that direct object type queries (not through an interface) are resolved as expected.
#[test]
fn direct_object_query_account_by_id() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            id
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          account(id: "a1") {
            id
          }
        }
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `chat` object field with a nested `account` object field.
/// Verifies that nested object fields are resolved correctly in the query plan.
#[test]
fn object_query_with_nested_object_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          chat(id: "c1") {
            id
            text
            account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            chat(id: "c1") {
              __typename
              id
              text
            }
          }
        },
        Flatten(path: "chat") {
          Fetch(service: "a") {
            {
              ... on Chat {
                __typename
                id
              }
            } =>
            {
              ... on Chat {
                account {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `account` object field with a nested `chats` list field.
/// Verifies that nested list fields are resolved correctly in the query plan.
#[test]
fn object_query_with_nested_list_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            id
            username
            chats {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            account(id: "a1") {
              __typename
              id
              username
            }
          }
        },
        Flatten(path: "account") {
          Fetch(service: "b") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                chats {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field for the `id` field directly.
/// Verifies that an error is returned if no subgraph resolves `id` for all object types implementing the interface.
#[test]
fn node_query_with_id_on_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          node(id: "a1") {
            id
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "b") {
          {
            node(id: "a1") {
              __typename
              ... on Chat {
                id
              }
            }
          }
        },
        Fetch(service: "a") {
          {
            node(id: "a1") {
              __typename
              ... on Account {
                id
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with multiple inline fragments for different possible types (`Chat` and `Account`).
/// Verifies that an error is returned if the required fields cannot be resolved for all possible types.
#[test]
fn node_query_with_multiple_type_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          node(id: "a1") {
            ... on Chat {
              id
            }
            ... on Account {
              id
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "a") {
          {
            node(id: "a1") {
              __typename
              ... on Account {
                id
              }
            }
          }
        },
        Fetch(service: "b") {
          {
            node(id: "a1") {
              __typename
              ... on Chat {
                id
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with both a direct `id` field and an inline fragment for a mismatched type.
/// Verifies that an error is returned if the required fields cannot be resolved for all possible types.
#[test]
fn node_query_with_id_and_cross_type_fragment_overlap() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            id
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            __typename
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "b") {
          {
            account: node(id: "a1") {
              __typename
              ... on Chat {
                id
              }
            }
          }
        },
        Fetch(service: "a") {
          {
            account: node(id: "a1") {
    ...a        }
            chat: node(id: "c1") {
    ...a        }
          }
          fragment a on Node {
            __typename
            ... on Account {
              id
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn type_expand_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            id
            reviews {
              id
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan("fixture/tests/abstract-types.supergraph.graphql", document)?;

    // TODO: this should be batched (turned into a sequence of two fetch steps)
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              id
              ... on Book {
                __typename
                id
              }
              ... on Magazine {
                __typename
                id
              }
            }
          }
        },
        Parallel {
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  reviews {
                    id
                  }
                }
              }
            },
          },
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Book {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  reviews {
                    id
                  }
                }
              }
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn poop1() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($title: Boolean = true) {
          products {
            id
            reviews {
              product {
                id
                ... on Book @include(if: $title) {
                  title
                }
                ... on Magazine {
                  sku
                }
              }
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan("fixture/tests/abstract-types.supergraph.graphql", document)?;

    // TODO: this should be batched (turned into a sequence of two fetch steps)
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              id
              ... on Book {
                __typename
                id
              }
              ... on Magazine {
                __typename
                id
              }
            }
          }
        },
        Parallel {
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  reviews {
                    product {
                      __typename
                      id
                      ... on Book {
                        __typename
                        id
                      }
                      ... on Magazine {
                        __typename
                        id
                      }
                    }
                  }
                }
              }
            },
          },
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Book {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  reviews {
                    product {
                      __typename
                      id
                      ... on Book {
                        __typename
                        id
                      }
                      ... on Magazine {
                        __typename
                        id
                      }
                    }
                  }
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "products.@.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  sku
                }
              }
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "books") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    title
                  }
                }
              },
            },
          },
          Flatten(path: "products.@.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  sku
                }
              }
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "books") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    title
                  }
                }
              },
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn poop2() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($title: Boolean = true) {
          products {
            id
            reviews {
              product {
                id
                ... on Book @include(if: $title) {
                  title
                  ... on Book {
                    sku
                  }
                }
                ... on Magazine {
                  sku
                }
              }
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan("fixture/tests/abstract-types.supergraph.graphql", document)?;

    // TODO: this should be batched (turned into a sequence of two fetch steps)
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              id
              ... on Book {
                __typename
                id
              }
              ... on Magazine {
                __typename
                id
              }
            }
          }
        },
        Parallel {
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  reviews {
                    product {
                      __typename
                      id
                      ... on Book {
                        __typename
                        id
                      }
                      ... on Magazine {
                        __typename
                        id
                      }
                    }
                  }
                }
              }
            },
          },
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Book {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  reviews {
                    product {
                      __typename
                      id
                      ... on Book {
                        __typename
                        id
                      }
                      ... on Magazine {
                        __typename
                        id
                      }
                    }
                  }
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "products.@.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  sku
                }
              }
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "products") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    sku
                  }
                }
              },
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "books") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    title
                  }
                }
              },
            },
          },
          Flatten(path: "products.@.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Magazine {
                  sku
                }
              }
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "products") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    sku
                  }
                }
              },
            },
          },
          Include(if: $title) {
            Flatten(path: "products.@.reviews.@.product") {
              Fetch(service: "books") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    title
                  }
                }
              },
            },
          },
        },
      },
    },
    "#);

    Ok(())
}
