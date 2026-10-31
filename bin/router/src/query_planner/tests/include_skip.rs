use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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

    let query_plan =
        build_query_plan_with_defaults("fixture/products-example.supergraph.graphql", document);
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

    let query_plan =
        build_query_plan_with_defaults("fixture/products-example.supergraph.graphql", document);
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
    let query_plan = build_query_plan_with_defaults(
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
fn qp_include_field_level_unconditional_before_conditional() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            isCheap # graph B - requires `price`
            name @include(if: $bool) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
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
                ... on Product @include(if: $bool) {
                  name
                }
                isCheap
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
fn qp_include_field_level_all_merged_fields_conditional() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            name @include(if: $bool) # graph B - requires `price`
            isCheap @include(if: $bool) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
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
                  isCheap
                  name
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
fn qp_skip_field_level() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            name @skip(if: $bool) # graph B - requires `price`
            isCheap # graph B - requires `price`
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
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

#[test]
fn qp_skip_field_level_unconditional_before_conditional() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            isCheap # graph B - requires `price`
            name @skip(if: $bool) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
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
                ... on Product @skip(if: $bool) {
                  name
                }
                isCheap
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
fn qp_skip_field_level_all_merged_fields_conditional() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($bool: Boolean) {
          product {
            price # graph A
            name @skip(if: $bool) # graph B - requires `price`
            isCheap @skip(if: $bool) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
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
        Skip(if: $bool) {
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
                  isCheap
                  name
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
fn qp_skip_and_include_field_level_all_merged_fields_conditional() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($skip: Boolean, $include: Boolean) {
          product {
            price # graph A
            name @skip(if: $skip) @include(if: $include) # graph B - requires `price`
            isCheap @skip(if: $skip) @include(if: $include) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($include:Boolean,$skip:Boolean) {
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
                    isCheap
                    name
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
fn qp_mixed_include_skip_field_level_keeps_conditions_scoped() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($include: Boolean, $skip: Boolean) {
          product {
            price # graph A
            name @include(if: $include) # graph B - requires `price`
            isCheap @skip(if: $skip) # graph B - requires `price`
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($include:Boolean,$skip:Boolean) {
            product {
              __typename
              price
              id
              ... on Product @include(if: $include) {
                ...a
              }
              ... on Product @skip(if: $skip) {
                ...a
              }
            }
          }
          fragment a on Product {
            price
            __typename
            id
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
            ($include:Boolean,$skip:Boolean) {
              ... on Product {
                ... on Product @skip(if: $skip) {
                  isCheap
                }
                ... on Product @include(if: $include) {
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
fn qp_nested_include_skip_conditions_in_complex_products_query() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($reviews: Boolean, $skipAuthor: Boolean, $nestedReviews: Boolean, $product: Boolean) {
          topProducts {
            name
            reviews @include(if: $reviews) {
              body
              author @skip(if: $skipAuthor) {
                name
                reviews @include(if: $nestedReviews) {
                  body
                  product @include(if: $product) {
                    name
                    shippingEstimate
                  }
                }
              }
            }
          }
        }
      "#,
    );

    let query_plan =
        build_query_plan_with_defaults("fixture/products-example.supergraph.graphql", document)?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              __typename
              name
              upc
            }
          }
        },
        Include(if: $reviews) {
          Sequence {
            Flatten(path: "topProducts.@") {
              Fetch(service: "reviews") {
                {
                  ... on Product {
                    __typename
                    upc
                  }
                } =>
                ($nestedReviews:Boolean,$product:Boolean,$skipAuthor:Boolean) {
                  ... on Product {
                    reviews {
                      body
                      author @skip(if: $skipAuthor) {
                        __typename
                        id
                        reviews @include(if: $nestedReviews) {
                          body
                          product @include(if: $product) {
                            __typename
                            upc
                          }
                        }
                      }
                    }
                  }
                }
              },
            },
            Skip(if: $skipAuthor) {
              Parallel {
                Include(if: $nestedReviews) {
                  Include(if: $product) {
                    Flatten(path: "topProducts.@.reviews.@.author.reviews.@.product") {
                      Fetch(service: "products") {
                        {
                          ... on Product {
                            __typename
                            upc
                          }
                        } =>
                        {
                          ... on Product {
                            price
                            weight
                            name
                          }
                        }
                      },
                    },
                  },
                },
                Flatten(path: "topProducts.@.reviews.@.author") {
                  Fetch(service: "accounts") {
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
            },
            Skip(if: $skipAuthor) {
              Include(if: $nestedReviews) {
                Include(if: $product) {
                  Flatten(path: "topProducts.@.reviews.@.author.reviews.@.product") {
                    Fetch(service: "inventory") {
                      {
                        ... on Product {
                          __typename
                          price
                          weight
                          upc
                        }
                      } =>
                      {
                        ... on Product {
                          shippingEstimate
                        }
                      }
                    },
                  },
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
fn qp_abstract_interface_mixed_conditions_stay_scoped() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($showReviews: Boolean, $hideSku: Boolean) {
          products {
            id
            sku @skip(if: $hideSku)
            reviewsCount @include(if: $showReviews)
            ... on Book @include(if: $showReviews) {
              title
            }
            ... on Magazine @skip(if: $hideSku) {
              title
            }
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          query ($hideSku:Boolean) {
            products {
              id
              __typename
              sku @skip(if: $hideSku)
            }
          }
        },
        Parallel {
          Include(if: $showReviews) {
            Parallel {
              Flatten(path: "products.@") {
                Fetch(service: "reviews") {
                  {
                    ... on Book {
                      __typename
                      id
                    }
                    ... on Magazine {
                      __typename
                      id
                    }
                  } =>
                  {
                    ... on Book {
                      reviewsCount
                    }
                    ... on Magazine {
                      reviewsCount
                    }
                  }
                },
              },
              Flatten(path: "products.@|[Book]") {
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
          Skip(if: $hideSku) {
            Flatten(path: "products.@|[Magazine]") {
              Fetch(service: "magazines") {
                {
                  ... on Magazine {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Magazine {
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
fn qp_abstract_interface_shared_condition_can_skip_remote_fetch() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($showReviews: Boolean) {
          products {
            id
            reviewsCount @include(if: $showReviews)
            reviewsScore @include(if: $showReviews)
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              id
              __typename
            }
          }
        },
        Include(if: $showReviews) {
          Flatten(path: "products.@") {
            Fetch(service: "reviews") {
              {
                ... on Book {
                  __typename
                  id
                }
                ... on Magazine {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  reviewsCount
                  reviewsScore
                }
                ... on Magazine {
                  reviewsCount
                  reviewsScore
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
fn qp_abstract_union_member_conditions_stay_scoped() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query ($showUserReview: Boolean, $hideAnonymousReview: Boolean) {
          review {
            ... on UserReview @include(if: $showUserReview) {
              product {
                b
              }
            }
            ... on AnonymousReview @skip(if: $hideAnonymousReview) {
              product {
                c
              }
            }
          }
        }
      "#,
    );

    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/union-overfetching.supergraph.graphql",
        document,
    )?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($hideAnonymousReview:Boolean,$showUserReview:Boolean) {
            review {
              __typename
              ... on UserReview {
                product @include(if: $showUserReview) {
                  ...a
                }
              }
              ... on AnonymousReview {
                product @skip(if: $hideAnonymousReview) {
                  ...a
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Parallel {
          Skip(if: $hideAnonymousReview) {
            Flatten(path: "review|[AnonymousReview].product") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Product {
                    c
                  }
                }
              },
            },
          },
          Include(if: $showUserReview) {
            Flatten(path: "review|[UserReview].product") {
              Fetch(service: "b") {
                {
                  ... on Product {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Product {
                    b
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

/// Reduced from generated `override_requires` seed 33 (and `interface-object-with-requires`
/// seed 110). Both `@requires` fields send entity calls whose paths go through
/// `... on Query @include(if: $x)`. Merging the first one into the root fetch lifted that
/// condition to the whole fetch and dropped the fragment, so the second one had no path left to
/// land on.
#[test]
fn conditional_root_fragment_survives_merges() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($x: Boolean!) {
          ... on Query @include(if: $x) {
            userInB {
              aName
              cName
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          query ($x:Boolean!) {
            ... on Query @include(if: $x) {
              userInB {
                __typename
                id
                ... on User @include(if: $x) {
                  name
                  __typename
                  id
                }
              }
            }
          }
        },
        Include(if: $x) {
          Parallel {
            Flatten(path: "|[Query].userInB") {
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
            Flatten(path: "|[Query].userInB") {
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
    },
    "#);

    Ok(())
}

/// The `b` call for `name` sits under two fragments, `@include(if: $a)` and `@include(if: $b)`,
/// so it has to be skipped unless both are true.
#[test]
fn entity_call_under_nested_conditional_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($a: Boolean!, $b: Boolean!) {
          userInA {
            id
            ... on User @include(if: $a) {
              ... on User @include(if: $b) {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($a:Boolean!,$b:Boolean!) {
            userInA {
              id
              ... on User @include(if: $a) {
                ... on User @include(if: $b) {
                  __typename
                  id
                }
              }
            }
          }
        },
        Include(if: $a) {
          Include(if: $b) {
            Flatten(path: "userInA|[User]|[User]") {
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
        },
      },
    },
    "#);

    Ok(())
}

/// `notes` and `price` are in `... on Product @include(if: $a)`, and `reviews` only sends the
/// `__typename` that the `products` call needs when `$a` is true. `price` has its own condition
/// too, so it gets its own call, and batching the two calls into one used to drop `$a`.
#[test]
fn batched_entity_calls_keep_the_condition_of_their_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($a: Boolean!, $b: Boolean!) {
          users {
            reviews {
              product {
                upc
                ... on Product @include(if: $a) {
                  notes
                  price @include(if: $b)
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{query_plan}"), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "accounts") {
          {
            users {
              __typename
              id
            }
          }
        },
        Flatten(path: "users.@") {
          Fetch(service: "reviews") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($a:Boolean!) {
              ... on User {
                reviews {
                  product {
                    upc
                    ... on Product @include(if: $a) {
                      __typename
                      upc
                    }
                  }
                }
              }
            }
          },
        },
        Include(if: $a) {
          Flatten(path: "users.@.reviews.@.product|[Product]") {
            Fetch(service: "products") {
              {
                ... on Product {
                  __typename
                  upc
                }
              } =>
              ($a:Boolean!,$b:Boolean!) {
                ... on Product {
                  ... on Product @include(if: $b) {
                    price
                  }
                  ... on Product @include(if: $a) {
                    notes
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

/// `F` is spread under `@include(if: $x)` and wraps a `... on Dog @skip(if: $y)`. `tricks` needs
/// both, the spread's `$x` can't get lost on the way.
#[test]
fn spread_condition_kept_around_same_type_fragment_with_its_own() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($x: Boolean!, $y: Boolean!) {
          listings {
            pet {
              ...F @include(if: $x)
            }
          }
        }

        fragment F on Dog {
          ... on Dog @skip(if: $y) {
            tricks
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "search") {
          query ($x:Boolean!,$y:Boolean!) {
            listings {
              pet {
                __typename
                ... on Dog @include(if: $x) {
                  ... on Dog @skip(if: $y) {
                    __typename
                    id
                  }
                }
              }
            }
          }
        },
        Include(if: $x) {
          Skip(if: $y) {
            Flatten(path: "listings.@.pet|[Dog]|[Dog]") {
              Fetch(service: "catalog") {
                {
                  ... on Dog {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Dog {
                    tricks
                  }
                }
              },
            },
          },
        },
      },
    },
    "#);

    // The same with a field: `name` has `$y` and needs `$x` too
    let document = parse_operation(
        r#"
        query($x: Boolean!, $y: Boolean!) {
          listings {
            pet {
              ... on Dog @include(if: $x) {
                name @skip(if: $y)
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "search") {
          query ($x:Boolean!) {
            listings {
              pet {
                __typename
                ... on Dog @include(if: $x) {
                  __typename
                  id
                }
              }
            }
          }
        },
        Include(if: $x) {
          Skip(if: $y) {
            Flatten(path: "listings.@.pet|[Dog]") {
              Fetch(service: "catalog") {
                {
                  ... on Dog {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Dog {
                    name
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

/// The Book and Magazine calls below `similar|[Book].reviews` get batched. `[Book]` is about
/// `similar`, not about the products, so `$a` has to gate the Magazines too.
#[test]
fn condition_kept_on_every_type_of_a_batched_call_under_a_type_condition(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!) {
          similar(id: "x") {
            reviews @include(if: $a) {
              product {
                delivery {
                  fastestDelivery
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            similar(id: "x") {
              __typename
              id
            }
          }
        },
        Include(if: $a) {
          Sequence {
            Flatten(path: "similar.@") {
              Fetch(service: "reviews") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                  ... on Magazine {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    reviews {
                      ...a
                    }
                  }
                  ... on Magazine {
                    reviews {
                      ...a
                    }
                  }
                }
                fragment a on Review {
                  product {
                    __typename
                    id
                  }
                }
              },
            },
            Flatten(path: "similar.@.reviews.@.product") {
              Fetch(service: "products") {
                {
                  ... on Book {
                    __typename
                    id
                  }
                  ... on Magazine {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Book {
                    dimensions {
                      ...a
                    }
                  }
                  ... on Magazine {
                    dimensions {
                      ...a
                    }
                  }
                }
                fragment a on ProductDimension {
                  weight
                  size
                }
              },
            },
            Flatten(path: "similar.@.reviews.@.product") {
              Fetch(service: "inventory") {
                {
                  ... on Book {
                    __typename
                    dimensions {
                      size
                      weight
                    }
                    id
                  }
                  ... on Magazine {
                    __typename
                    dimensions {
                      size
                      weight
                    }
                    id
                  }
                } =>
                {
                  ... on Book {
                    delivery {
                      ...a
                    }
                  }
                  ... on Magazine {
                    delivery {
                      ...a
                    }
                  }
                }
                fragment a on DeliveryEstimates {
                  fastestDelivery
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

/// `orders` comes under `$a` and under `$c` both. The `name` call has its own `$b` and was planned
/// under `$a` too: with `$a` off the orders still come from the `$c` branch, so it needs both.
/// The `weight` it requires is fetched for that branch as well.
#[test]
fn condition_kept_when_another_branch_fetches_the_same_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!, $c: Boolean!) {
          user {
            orders @include(if: $a) {
              ... on Order @include(if: $b) {
                name
              }
            }
            ... @skip(if: $c) {
              orders {
                weight
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-partly-provided-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          query ($c:Boolean!) {
            user {
              __typename
              id
              ... on User @skip(if: $c) {
                __typename
                id
              }
            }
          }
        },
        BatchFetch(service: "orders") {
          {
            _e0 {
              paths: [
                "user|[User]"
              ]
              {
                ... on User {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "user"
              ]
              {
                ... on User {
                  __typename
                  id
                }
              }
            }
          }
          ($c:Boolean!,$b:Boolean!,$a:Boolean!) {
            _e0: _entities(representations: $__batch_reps_0) @skip(if: $c) {
              ... on User {
                orders {
                  __typename
                  id
                }
              }
            }
            _e1: _entities(representations: $__batch_reps_1) @include(if: $a) {
              ... on User {
                orders {
                  ... on Order @include(if: $b) {
                    __typename
                    id
                    sku
                  }
                }
              }
            }
          }
        },
        Parallel {
          Skip(if: $c) {
            Flatten(path: "user|[User].orders.@") {
              Fetch(service: "catalog") {
                {
                  ... on Order {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Order {
                    weight
                  }
                }
              },
            },
          },
          Include(if: $a) {
            Include(if: $b) {
              Flatten(path: "user.orders.@|[Order]") {
                Fetch(service: "catalog") {
                  {
                    ... on Order {
                      __typename
                      id
                    }
                  } =>
                  {
                    ... on Order {
                      weight
                    }
                  }
                },
              },
            },
          },
        },
        Include(if: $a) {
          Include(if: $b) {
            Flatten(path: "user.orders.@|[Order]") {
              Fetch(service: "orders") {
                {
                  ... on Order {
                    __typename
                    sku
                    weight
                    id
                  }
                } =>
                {
                  ... on Order {
                    name
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

/// `orders` is asked twice, once under `@include(if: $a)`. With `$a` off, the unconditional one
/// still has the orders, so the calls for `name` need `$a` as well as their own `$b`.
#[test]
fn field_condition_kept_when_the_field_is_also_unconditional() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!) {
          user {
            orders {
              id
            }
            orders @include(if: $a) {
              ... on Order @include(if: $b) {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-partly-provided-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "orders") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($a:Boolean!,$b:Boolean!) {
              ... on User {
                ... on User @include(if: $a) {
                  orders {
                    ... on Order @include(if: $b) {
                      __typename
                      id
                      sku
                    }
                  }
                }
                orders {
                  id
                }
              }
            }
          },
        },
        Include(if: $a) {
          Include(if: $b) {
            Sequence {
              Flatten(path: "user.orders.@|[Order]") {
                Fetch(service: "catalog") {
                  {
                    ... on Order {
                      __typename
                      id
                    }
                  } =>
                  {
                    ... on Order {
                      weight
                    }
                  }
                },
              },
              Flatten(path: "user.orders.@|[Order]") {
                Fetch(service: "orders") {
                  {
                    ... on Order {
                      __typename
                      sku
                      weight
                      id
                    }
                  } =>
                  {
                    ... on Order {
                      name
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

/// `... on Animal` on an `Animal` doesn't narrow anything, its condition goes onto `... on Dog`.
/// Otherwise the Dogs at `pet|[Animal]|[Dog]` and at `pet|[Dog]` are looked up twice.
#[test]
fn condition_on_a_fragment_of_the_same_type_goes_to_its_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($x: Boolean!) {
          listings {
            pet {
              ... on Animal @include(if: $x) {
                ... on Dog {
                  tricks
                }
              }
              ... on Dog {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "search") {
          {
            listings {
              pet {
                __typename
                ... on Dog {
                  __typename
                  id
                }
              }
            }
          }
        },
        Flatten(path: "listings.@.pet|[Dog]") {
          Fetch(service: "catalog") {
            {
              ... on Dog {
                __typename
                id
              }
            } =>
            ($x:Boolean!) {
              ... on Dog {
                name
                ... on Dog @include(if: $x) {
                  tricks
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

/// The fetch below `@skip(if: $c) @include(if: $d)` was planned under `$a` and `$b` too. `$a`
/// sits on `anotherUsers` and the list has no spot for `$b`, it still has to wrap the fetch.
#[test]
fn condition_with_no_spot_on_the_path_still_wraps_the_fetch() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($a: Boolean!, $b: Boolean!, $c: Boolean!, $d: Boolean!) {
          anotherUsers @skip(if: $a) {
            ... @skip(if: $b) {
              ... @skip(if: $c) @include(if: $d) {
                __typename
              }
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          query ($a:Boolean!,$b:Boolean!) {
            anotherUsers @skip(if: $a) {
              __typename
              ... on NodeWithName @skip(if: $b) {
                __typename
                id
              }
            }
          }
        },
        Skip(if: $a) {
          Sequence {
            Skip(if: $b) {
              Sequence {
                Flatten(path: "anotherUsers.@") {
                  Fetch(service: "a") {
                    {
                      ... on NodeWithName {
                        __typename
                        id
                      }
                    } =>
                    {
                      ... on NodeWithName {
                        __typename
                      }
                    }
                  },
                },
                Flatten(path: "anotherUsers.@") {
                  Fetch(service: "b") {
                    {
                      ... on User {
                        __typename
                        id
                      }
                    } =>
                    {
                      ... on NodeWithName {
                        __typename
                        id
                      }
                    }
                  },
                },
              },
            },
            Skip(if: $c) {
              Include(if: $d) {
                Skip(if: $b) {
                  Flatten(path: "anotherUsers.@") {
                    Fetch(service: "a") {
                      {
                        ... on NodeWithName {
                          __typename
                          id
                        }
                      } =>
                      {
                        ... on NodeWithName {
                          __typename
                          ... on User {
                            __typename
                          }
                        }
                      }
                    },
                  },
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
