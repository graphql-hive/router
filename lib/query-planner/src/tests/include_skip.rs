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

    let query_plan = build_query_plan(
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

    let query_plan = build_query_plan(
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

    let query_plan = build_query_plan(
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

    let query_plan = build_query_plan("fixture/products-example.supergraph.graphql", document)?;
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
        },
        Parallel {
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
          Skip(if: $skipAuthor) {
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

    let query_plan = build_query_plan("fixture/tests/abstract-types.supergraph.graphql", document)?;
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
            Flatten(path: "products.@") {
              Fetch(service: "reviews") {
                {
                  ... on Product {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Product {
                    reviewsCount
                  }
                }
              },
            },
          },
          Include(if: $showReviews) {
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

    let query_plan = build_query_plan("fixture/tests/abstract-types.supergraph.graphql", document)?;
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
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
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

    let query_plan = build_query_plan(
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

/// Regression: unconditional `username` must not disappear when an aliased `@skip` copy remains
#[test]
fn qp_nested_user_username_skip_and_unconditional_preserved() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($flag: Boolean!, $id: ID!) {
          user(id: $id) {
            reviews {
              author {
                ... on User @include(if: true) {
                  reviews {
                    ... on Review @include(if: true) @skip(if: false) {
                      id
                      author {
                        ... on User {
                          __typename
                          masked: username @skip(if: $flag)
                          username @include(if: true) @skip(if: false)
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
    );
    let query_plan = build_query_plan("../../bench/supergraph.graphql", document)?;
    let printed = format!("{}", query_plan);

    insta::assert_snapshot!(printed, @r###"
    QueryPlan {
      Sequence {
        Fetch(service: "accounts") {
          query ($id:ID!) {
            user(id: $id) {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "reviews") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($flag:Boolean!) {
              ... on User {
                reviews {
                  author {
                    reviews {
                      id
                      author {
                        __typename
                        masked: username @skip(if: $flag)
                        username
                      }
                    }
                  }
                }
              }
            }
          },
        },
      },
    },
    "###);

    Ok(())
}
