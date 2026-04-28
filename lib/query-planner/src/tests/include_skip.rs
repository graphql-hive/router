use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn include_basic_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price
            neverCalledInclude @include(if: $bool)
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($bool:Boolean) {
            product {
              __typename
              price
              id
              ... on Product @include(if: $bool) {
                price
                __typename
                id
              }
            }
          }
        },
        Include(if: $bool) {
          Sequence {
            Flatten(path: "product") {
              Fetch(service: "b") {
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                } =>
                {
                  ... on Product {
                    isExpensive
                  }
                }
              },
            },
            Flatten(path: "product") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    neverCalledInclude
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
fn include_fragment_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price
            ... on Product @include(if: $bool) {
              neverCalledInclude
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($bool:Boolean) {
            product {
              price
              ... on Product @include(if: $bool) {
                __typename
                id
                price
              }
            }
          }
        },
        Include(if: $bool) {
          Sequence {
            Flatten(path: "product|[Product]") {
              Fetch(service: "b") {
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                } =>
                {
                  ... on Product {
                    isExpensive
                  }
                }
              },
            },
            Flatten(path: "product|[Product]") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    neverCalledInclude
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
fn skip_basic_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean = false) {
          product {
            price
            skip @skip(if: $bool)
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($bool:Boolean=false) {
            product {
              __typename
              price
              id
              ... on Product @skip(if: $bool) {
                price
                __typename
                id
              }
            }
          }
        },
        Skip(if: $bool) {
          Sequence {
            Flatten(path: "product") {
              Fetch(service: "b") {
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                } =>
                {
                  ... on Product {
                    isExpensive
                  }
                }
              },
            },
            Flatten(path: "product") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    skip
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
fn skip_and_include_field_condition_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean = false, $include: Boolean = true) {
          product {
            price
            neverCalledInclude @skip(if: $skip) @include(if: $include)
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($include:Boolean=true,$skip:Boolean=false) {
            product {
              __typename
              price
              id
              ... on Product @skip(if: $skip) @include(if: $include) {
                price
                __typename
                id
              }
            }
          }
        },
        Skip(if: $skip) {
          Include(if: $include) {
            Sequence {
              Flatten(path: "product") {
                Fetch(service: "b") {
                  {
                    ... on Product {
                      __typename
                      price
                      id
                    }
                  } =>
                  {
                    ... on Product {
                      isExpensive
                    }
                  }
                },
              },
              Flatten(path: "product") {
                Fetch(service: "c") {
                  {
                    ... on Product {
                      __typename
                      isExpensive
                      id
                    }
                  } =>
                  {
                    ... on Product {
                      neverCalledInclude
                    }
                  }
                },
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
fn skip_and_include_fragment_condition_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean = false, $include: Boolean = true) {
          product {
            price
            ... on Product @skip(if: $skip) @include(if: $include) {
              neverCalledInclude
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($include:Boolean=true,$skip:Boolean=false) {
            product {
              price
              ... on Product @skip(if: $skip) @include(if: $include) {
                __typename
                id
                price
              }
            }
          }
        },
        Skip(if: $skip) {
          Include(if: $include) {
            Sequence {
              Flatten(path: "product|[Product]") {
                Fetch(service: "b") {
                  {
                    ... on Product {
                      __typename
                      price
                      id
                    }
                  } =>
                  {
                    ... on Product {
                      isExpensive
                    }
                  }
                },
              },
              Flatten(path: "product|[Product]") {
                Fetch(service: "c") {
                  {
                    ... on Product {
                      __typename
                      isExpensive
                      id
                    }
                  } =>
                  {
                    ... on Product {
                      neverCalledInclude
                    }
                  }
                },
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
fn include_at_root_fetch_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            id
            price @include(if: $bool)
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($bool:Boolean) {
          product {
            id
            price @include(if: $bool)
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn include_fragment_at_root_fetch_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            id
            ... on Product @include(if: $bool) {
              price
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($bool:Boolean) {
          product {
            id
            ... on Product @include(if: $bool) {
              price
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn include_interface_at_root_fetch_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          accounts {
            id
            name @include(if: $bool)
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        query ($bool:Boolean) {
          accounts {
            id
            name @include(if: $bool)
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn include_interface_fragment_at_root_fetch_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          accounts {
            id
            ... on Account @include(if: $bool) {
              name
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        query ($bool:Boolean) {
          accounts {
            id
            ... on Account @include(if: $bool) {
              name
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn include_union_fragment_at_root_fetch_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          review {
            ... on UserReview @include(if: $bool) {
              product {
                id
              }
            }
            ... on AnonymousReview @include(if: $bool) {
              product {
                id
              }
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/union-overfetching.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($bool:Boolean) {
          review {
            __typename
            ... on UserReview {
              product @include(if: $bool) {
                ...a
              }
            }
            ... on AnonymousReview {
              product @include(if: $bool) {
                ...a
              }
            }
          }
        }
        fragment a on Product {
          id
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn plans_query_with_nested_directive_only_inline_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        fragment User on User {
          id
          username
          name
        }

        fragment Review on Review {
          id
          body
        }

        fragment Product on Product {
          inStock
          name
          price
          shippingEstimate
          upc
          weight
        }

        query TestQuery($user: Boolean = true, $product: Boolean = true, $reviews: Boolean = true) {
          users {
            ...User
            ... @include(if: $reviews) {
              reviews {
                ...Review
                product {
                  ...Product
                  reviews {
                    ...Review
                    ... @include(if: $user) {
                      author {
                        ...User
                        reviews {
                          ...Review
                          ... @include(if: $product) {
                            product {
                              ...Product
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
          ... @include(if: $product) {
            topProducts {
              ...Product
              reviews {
                ...Review
                author {
                  ...User
                  ... @include(if: $reviews) {
                    reviews {
                      ...Review
                      product {
                        ...Product
                      }
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );

    let query_plan = build_query_plan("fixture/products-example.supergraph.graphql", document);
    assert!(
        query_plan.is_ok(),
        "expected query planning to succeed, got: {:?}",
        query_plan.err()
    );

    Ok(())
}

#[test]
fn plans_query_with_field_level_include_skip_conditions() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        fragment User on User {
          id
          username
          name
        }

        fragment Review on Review {
          id
          body
        }

        fragment Product on Product {
          inStock
          name
          price
          shippingEstimate
          upc
          weight
        }

        query TestQuery($user: Boolean = true, $product: Boolean = true, $reviews: Boolean = true) {
          users {
            ...User
            reviews @include(if: $reviews) {
              ...Review
              product {
                ...Product
                reviews {
                  ...Review
                  author @include(if: $user) {
                    ...User
                    reviews {
                      ...Review
                      product @include(if: $product) {
                        ...Product
                      }
                    }
                  }
                }
              }
            }
          }
          topProducts @include(if: $product) {
            ...Product
            reviews {
              ...Review
              author {
                ...User
                reviews @include(if: $reviews) {
                  ...Review
                  product {
                    ...Product
                  }
                }
              }
            }
          }
        }
        "#,
    );

    let query_plan = build_query_plan("fixture/products-example.supergraph.graphql", document);
    assert!(
        query_plan.is_ok(),
        "expected query planning to succeed, got: {:?}",
        query_plan.err()
    );

    Ok(())
}

#[test]
fn qp_include_field_level() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            name @include(if: $bool) # graph B
            isCheap # graph B
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($bool:Boolean) {
            product {
              __typename
              price
              id
              ... on Product @include(if: $bool) {
                price
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            ($bool:Boolean) {
              ... on Product {
                isCheap
                ... on Product @include(if: $bool) {
                  name
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

#[test]
fn qp_skip_field_level() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            name @skip(if: $bool) # graph B
            isCheap # graph B
          }
        }
      "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($bool:Boolean) {
            product {
              __typename
              price
              id
              ... on Product @skip(if: $bool) {
                price
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            ($bool:Boolean) {
              ... on Product {
                isCheap
                ... on Product @skip(if: $bool) {
                  name
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
