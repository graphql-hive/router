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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        { product { __typename price id ... on Product @include(if: $bool) { __typename price id } } }
      vars
        $bool

    [6] b   after [2]
      if
        $bool
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] c   after [6]
      if
        $bool
      merge
        product
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { neverCalledInclude }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        { product { price ... on Product @include(if: $bool) { __typename id price } } }
      vars
        $bool

    [6] b   after [2]
      if
        $bool
      merge
        product|[Product]
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] c   after [6]
      if
        $bool
      merge
        product|[Product]
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { neverCalledInclude }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean = false

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        { product { __typename price id ... on Product @skip(if: $bool) { __typename price id } } }
      vars
        $bool

    [6] b   after [2]
      if
        !$bool
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] c   after [6]
      if
        !$bool
      merge
        product
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { skip }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $include: Boolean = true, $skip: Boolean = false

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        {
          product {
            __typename
            price
            id
            ... on Product @skip(if: $skip) @include(if: $include) { __typename price id }
          }
        }
      vars
        $include, $skip

    [6] b   after [2]
      if
        $include && !$skip
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] c   after [6]
      if
        $include && !$skip
      merge
        product
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { neverCalledInclude }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $include: Boolean = true, $skip: Boolean = false

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] c               [6]

    [2] a
      out
        {
          product {
            price
            ... on Product @skip(if: $skip) @include(if: $include) { __typename id price }
          }
        }
      vars
        $include, $skip

    [6] b   after [2]
      if
        $include && !$skip
      merge
        product|[Product]
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] c   after [6]
      if
        $include && !$skip
      merge
        product|[Product]
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { neverCalledInclude }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { product { id price @include(if: $bool) } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { product { id ... on Product @include(if: $bool) { price } } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { accounts { id name @include(if: $bool) } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { accounts { id ... on Account @include(if: $bool) { name } } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          review {
            __typename
            ... on UserReview { product @include(if: $bool) { ...a } }
            ... on AnonymousReview { product @include(if: $bool) { ...a } }
          }
        }
        fragment a on Product { id }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @include(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { ... on Product @include(if: $bool) { name } isCheap }
      vars
        $bool
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @include(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isCheap ... on Product @include(if: $bool) { name } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @include(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      if
        $bool
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { name isCheap }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @skip(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { ... on Product @skip(if: $bool) { name } isCheap }
      vars
        $bool
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @skip(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isCheap ... on Product @skip(if: $bool) { name } }
      vars
        $bool
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $bool: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { product { __typename price id ... on Product @skip(if: $bool) { __typename price id } } }
      vars
        $bool

    [4] b   after [2]
      if
        !$bool
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { name isCheap }
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $include: Boolean, $skip: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        {
          product {
            __typename
            price
            id
            ... on Product @skip(if: $skip) @include(if: $include) { __typename price id }
          }
        }
      vars
        $include, $skip

    [4] b   after [2]
      if
        $include && !$skip
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { name isCheap }
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $include: Boolean, $skip: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        {
          product {
            __typename
            price
            id
            ... on Product @skip(if: $skip) { ...a }
            ... on Product @include(if: $include) { ...a }
          }
        }
        fragment a on Product { __typename price id }
      vars
        $include, $skip

    [4] b   after [2]
      if
        !$skip || $include
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product {
          ... on Product @include(if: $include) { name }
          ... on Product @skip(if: $skip) { isCheap }
        }
      vars
        $include, $skip
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $nestedReviews: Boolean, $product: Boolean, $reviews, $skipAuthor: Boolean

    fetches
      fetch               after
      [2] products        -
      [3] reviews         [2]
      [8] products        [3]
      [4] accounts        [3]
      [7] inventory       [8]

    [2] products
      out
        { topProducts { __typename name upc } }

    [3] reviews   after [2]
      if
        $reviews
      merge
        topProducts.@
      in
        ... on Product { __typename upc }
      out
        ... on Product {
          reviews {
            body
            author @skip(if: $skipAuthor) {
              __typename
              id
              reviews @include(if: $nestedReviews) {
                body
                product @include(if: $product) { __typename upc }
              }
            }
          }
        }
      vars
        $nestedReviews, $product, $skipAuthor

    [8] products   after [3]
      if
        $nestedReviews && $product && $reviews && !$skipAuthor
      merge
        topProducts.@.reviews.@.author.reviews.@.product
      in
        ... on Product { __typename upc }
      out
        ... on Product { price weight name }

    [4] accounts   after [3]
      if
        $reviews && !$skipAuthor
      merge
        topProducts.@.reviews.@.author
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [7] inventory   after [8]
      if
        $nestedReviews && $product && $reviews && !$skipAuthor
      merge
        topProducts.@.reviews.@.author.reviews.@.product
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate }
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $hideSku: Boolean, $showReviews

    fetches
      fetch               after
      [2] products        -
      [6] reviews         [2]
      [5] books           [2]
      [3] magazines       [2]

    [2] products
      out
        { products { __typename id sku @skip(if: $hideSku) } }
      vars
        $hideSku

    [6] reviews   after [2]
      if
        $showReviews
      merge
        products.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviewsCount } ... on Magazine { reviewsCount }

    [5] books   after [2]
      if
        $showReviews
      merge
        products.@|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { title }

    [3] magazines   after [2]
      if
        !$hideSku
      merge
        products.@|[Magazine]
      in
        ... on Magazine { __typename id }
      out
        ... on Magazine { title }
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $showReviews

    fetches
      fetch               after
      [2] products        -
      [4] reviews         [2]

    [2] products
      out
        { products { __typename id } }

    [4] reviews   after [2]
      if
        $showReviews
      merge
        products.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviewsCount reviewsScore } ... on Magazine { reviewsCount reviewsScore }
    ");

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

    insta::assert_snapshot!(printed, @r"
    variables
      $hideAnonymousReview: Boolean, $showUserReview: Boolean

    fetches
      fetch               after
      [2] a               -
      [4] c               [2]
      [3] b               [2]

    [2] a
      out
        {
          review {
            __typename
            ... on UserReview { product @include(if: $showUserReview) { ...a } }
            ... on AnonymousReview { product @skip(if: $hideAnonymousReview) { ...a } }
          }
        }
        fragment a on Product { __typename id }
      vars
        $hideAnonymousReview, $showUserReview

    [4] c   after [2]
      if
        !$hideAnonymousReview
      merge
        review|[AnonymousReview].product
      in
        ... on Product { __typename id }
      out
        ... on Product { c }

    [3] b   after [2]
      if
        $showUserReview
      merge
        review|[UserReview].product
      in
        ... on Product { __typename id }
      out
        ... on Product { b }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!

    fetches
      fetch               after
      [2] b               -
      [4] a               [2]
      [7] c               [2]

    [2] b
      if
        $x
      out
        {
          ... on Query @include(if: $x) {
            userInB { __typename id ... on User @include(if: $x) { __typename name id } }
          }
        }
      vars
        $x

    [4] a   after [2]
      if
        $x
      merge
        |[Query].userInB
      in
        ... on User { __typename name id }
      out
        ... on User { aName }

    [7] c   after [2]
      if
        $x
      merge
        |[Query].userInB
      in
        ... on User { __typename name id }
      out
        ... on User { cName }
    ");

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

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        {
          userInA { id ... on User @include(if: $a) { ... on User @include(if: $b) { __typename id } } }
        }
      vars
        $a, $b

    [3] b   after [2]
      if
        $a && $b
      merge
        userInA|[User]|[User]
      in
        ... on User { __typename id }
      out
        ... on User { name }
    ");

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

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] accounts        -
      [3] reviews         [2]
      [5] products        [3]

    [2] accounts
      out
        { users { __typename id } }

    [3] reviews   after [2]
      merge
        users.@
      in
        ... on User { __typename id }
      out
        ... on User { reviews { product { upc ... on Product @include(if: $a) { __typename upc } } } }
      vars
        $a

    [5] products   after [3]
      if
        $a
      merge
        users.@.reviews.@.product|[Product]
      in
        ... on Product { __typename upc }
      out
        ... on Product {
          ... on Product @include(if: $b) { price }
          ... on Product @include(if: $a) { notes }
        }
      vars
        $a, $b
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a, $b: Boolean!, $c: Boolean!

    fetches
      fetch               after
      [2] users           -
      [6] orders          [2]
      [7] catalog         [6]
      [4] orders          [7]

    [2] users
      out
        { user { __typename id ... on User @skip(if: $c) { __typename id } } }
      vars
        $c

    [6] orders   after [2]
      alias _e0
        if
          !$c
        merge
          user|[User]
        in
          ... on User { __typename id }
        out
          ... on User { orders { __typename id } }
      alias _e1
        if
          $a
        merge
          user
        in
          ... on User { __typename id }
        out
          ... on User { orders { ... on Order @include(if: $b) { __typename id sku } } }
      vars
        $b

    [7] catalog   after [6]
      alias _e0
        if
          !$c
        merge
          user|[User].orders.@
        in
          ... on Order { __typename id }
        out
          ... on Order { ...a }
      alias _e1
        if
          $a && $b
        merge
          user.orders.@|[Order]
        in
          ... on Order { __typename id }
        out
          ... on Order { ...a }
      fragments
        fragment a on Order { weight }

    [4] orders   after [7]
      if
        $a && $b
      merge
        user.orders.@|[Order]
      in
        ... on Order { __typename sku weight id }
      out
        ... on Order { name }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] users           -
      [4] orders          [2]
      [6] catalog         [4]
      [5] orders          [6]

    [2] users
      out
        { user { __typename id } }

    [4] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User {
          ... on User @include(if: $a) {
            orders { ... on Order @include(if: $b) { __typename id sku } }
          }
          orders { id }
        }
      vars
        $a, $b

    [6] catalog   after [4]
      if
        $a && $b
      merge
        user.orders.@|[Order]
      in
        ... on Order { __typename id }
      out
        ... on Order { weight }

    [5] orders   after [6]
      if
        $a && $b
      merge
        user.orders.@|[Order]
      in
        ... on Order { __typename sku weight id }
      out
        ... on Order { name }
    ");
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
    variables
      $a: Boolean!, $b: Boolean!, $c, $d

    fetches
      fetch               after
      [2] b               -
      [5] a               [2]
      [3] b               [5]
      [6] a               [3]

    [2] b
      if
        !$a
      out
        {
          anotherUsers @skip(if: $a) { __typename ... on NodeWithName @skip(if: $b) { __typename id } }
        }
      vars
        $a, $b

    [5] a   after [2]
      if
        !$a && !$b
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { __typename }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [3] b   after [5]
      if
        !$a && !$b
      merge
        anotherUsers.@
      in
        ... on User { __typename id }
      out
        ... on NodeWithName { __typename id }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [6] a   after [3]
      if
        $d && !$a && !$b && !$c
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { __typename ... on User { __typename } }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!, $y: Boolean!

    fetches
      fetch               after
      [2] search          -
      [3] catalog         [2]

    [2] search
      out
        {
          listings {
            pet {
              __typename
              ... on Dog @include(if: $x) { ... on Dog @skip(if: $y) { __typename id } }
            }
          }
        }
      vars
        $x, $y

    [3] catalog   after [2]
      if
        $x && !$y
      merge
        listings.@.pet|[Dog]|[Dog]
      in
        ... on Dog { __typename id }
      out
        ... on Dog { tricks }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!, $y

    fetches
      fetch               after
      [2] search          -
      [3] catalog         [2]

    [2] search
      out
        { listings { pet { __typename ... on Dog @include(if: $x) { __typename id } } } }
      vars
        $x

    [3] catalog   after [2]
      if
        $x && !$y
      merge
        listings.@.pet|[Dog]
      in
        ... on Dog { __typename id }
      out
        ... on Dog { name }
    ");
    Ok(())
}

/// `tricks` sits under two `@include`s, which no single field can carry, so the entity call needs
/// both. Same with three levels of fragments, each one adds its condition.
#[test]
fn entity_call_under_nested_fragments_needs_every_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($x: Boolean!, $z: Boolean!) {
          listings {
            pet {
              ... on Dog @include(if: $x) {
                ... on Dog @include(if: $z) {
                  tricks
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!, $z: Boolean!

    fetches
      fetch               after
      [2] search          -
      [3] catalog         [2]

    [2] search
      out
        {
          listings {
            pet {
              __typename
              ... on Dog @include(if: $x) { ... on Dog @include(if: $z) { __typename id } }
            }
          }
        }
      vars
        $x, $z

    [3] catalog   after [2]
      if
        $x && $z
      merge
        listings.@.pet|[Dog]|[Dog]
      in
        ... on Dog { __typename id }
      out
        ... on Dog { tricks }
    ");

    let document = parse_operation(
        r#"
        query($x: Boolean!, $y: Boolean!, $z: Boolean!) {
          listings {
            pet {
              ... on Dog @include(if: $x) {
                ... on Dog @skip(if: $y) {
                  ... on Dog @include(if: $z) {
                    name
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!, $y: Boolean!, $z: Boolean!

    fetches
      fetch               after
      [2] search          -
      [3] catalog         [2]

    [2] search
      out
        {
          listings {
            pet {
              __typename
              ... on Dog @include(if: $x) {
                ... on Dog @skip(if: $y) @include(if: $z) { __typename id }
              }
            }
          }
        }
      vars
        $x, $y, $z

    [3] catalog   after [2]
      if
        $x && $z && !$y
      merge
        listings.@.pet|[Dog]|[Dog]
      in
        ... on Dog { __typename id }
      out
        ... on Dog { name }
    ");
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
    variables
      $a

    fetches
      fetch               after
      [2] products        -
      [10] reviews        [2]
      [6] products        [10]
      [15] inventory      [6]

    [2] products
      out
        { similar(id: "x") { __typename id } }

    [10] reviews   after [2]
      if
        $a
      merge
        similar.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviews { ...a } } ... on Magazine { reviews { ...a } }
        fragment a on Review { product { __typename id } }

    [6] products   after [10]
      if
        $a
      merge
        similar.@.reviews.@.product
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { dimensions { ...a } } ... on Magazine { dimensions { ...a } }
        fragment a on ProductDimension { weight size }

    [15] inventory   after [6]
      if
        $a
      merge
        similar.@.reviews.@.product
      in
        ... on Book { __typename dimensions { size weight } id }
        ... on Magazine { __typename dimensions { size weight } id }
      out
        ... on Book { delivery { ...a } } ... on Magazine { delivery { ...a } }
        fragment a on DeliveryEstimates { fastestDelivery }
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!

    fetches
      fetch               after
      [2] search          -
      [4] catalog         [2]

    [2] search
      out
        { listings { pet { __typename ... on Dog { __typename id } } } }

    [4] catalog   after [2]
      merge
        listings.@.pet|[Dog]
      in
        ... on Dog { __typename id }
      out
        ... on Dog { name ... on Dog @include(if: $x) { tricks } }
      vars
        $x
    ");

    Ok(())
}

#[test]
fn fetch_that_never_runs_is_left_out() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `reviews` needs `$a` to be true and false at once
    let document = parse_operation(
        r#"
        query ($a: Boolean!) {
          me {
            name
            ... @skip(if: $a) {
              ... @include(if: $a) {
                reviews {
                  body
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $a: Boolean!

    fetches
      fetch               after
      [2] accounts        -

    [2] accounts
      out
        { me { name ... on User @skip(if: $a) @include(if: $a) { __typename id } } }
      vars
        $a
    ");

    Ok(())
}

#[test]
fn plan_is_empty_when_no_fetch_ever_runs() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($a: Boolean!) {
          me @include(if: $a) @skip(if: $a) {
            name
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    fetches
      none
    ");

    Ok(())
}

/// `results` once on its own and once under `$a` with `... on Product { id }`. The projection
/// picks the `id` of a `Product` by `__typename`, so the subgraph has to send it with `$a` off
/// too, not only under the condition. The `results` that's always there when `recommender` is
/// gets it, the one under `$b` doesn't need it too.
#[test]
fn abstract_field_selected_again_under_a_condition_asks_for_its_typename(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!, $c: Boolean!) {
          recommender @include(if: $c) {
            results @include(if: $b) { id }
            results { id }
            results @include(if: $a) { ... on Product { id } }
          }
        }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/190.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!, $b: Boolean!, $c: Boolean!

    fetches
      fetch               after
      [2] recommender     -

    [2] recommender
      if
        $c
      out
        {
          recommender @include(if: $c) {
            results @include(if: $b) { id }
            results { __typename id }
            results @include(if: $a) { __typename ... on Product { id } }
          }
        }
      vars
        $a, $b, $c
    ");

    Ok(())
}

#[test]
fn abstract_field_selected_again_under_a_condition_in_an_entity_call_asks_for_its_typename(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!) {
          books {
            reviews { product { id } }
            reviews @include(if: $a) { product { ... on Book { id } } }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!

    fetches
      fetch               after
      [2] books           -
      [4] reviews         [2]

    [2] books
      out
        { books { __typename id } }

    [4] reviews   after [2]
      merge
        books.@
      in
        ... on Book { __typename id }
      out
        ... on Book {
          ... on Book @include(if: $a) { reviews { product { __typename ... on Book { id } } } }
          reviews { product { __typename id } }
        }
      vars
        $a
    ");

    Ok(())
}
