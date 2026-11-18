use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn requires_with_fragments_on_interfaces() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userFromA {
            permissions
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-fragments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        {
          userFromA {
            __typename
            id
            profile {
              __typename
              displayName
              ... on GuestAccount { guestToken accountType }
              ... on AdminAccount { adminLevel accountType }
            }
          }
        }

    [4] b   after [2]
      merge
        userFromA
      in
        ... on User {
          __typename
          profile {
            displayName
            ... on AdminAccount { accountType adminLevel }
            ... on GuestAccount { accountType guestToken }
          }
          id
        }
      out
        ... on User { permissions }
    ");

    Ok(())
}

#[test]
fn requires_with_union_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    // Media only lives in pricing, so `... on Media` has to keep Book there,
    // otherwise the whole requirement goes away and pricing never gets the title
    let document = parse_operation(
        r#"
        query {
          products {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-union-type-condition.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] catalog         -
      [4] pricing         [2]

    [2] catalog
      out
        { products { __typename id book { title } } }

    [4] pricing   after [2]
      merge
        products.@
      in
        ... on Product { __typename book { title } id }
      out
        ... on Product { price }
    ");

    Ok(())
}

#[test]
fn requires_root_fragment_on_inapplicable_type() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-inapplicable-root-fragment.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price upc }
      out
        ... on Product { isExpensive }
    ");

    Ok(())
}

/// `permissions @skip(if: $skip)` needs more of `profile` than `basicPermissions` does, and the
/// extra part is only fetched when `$skip` is false. In one shared call, `$skip: true` would send
/// a `profile` with fields missing, it'd become `null` and break `basicPermissions`. So they get
/// separate calls.
#[test]
fn conditional_requires_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-fragments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $skip: Boolean!

    fetches
      fetch               after
      [2] a               -
      [8] b               [2]

    [2] a
      out
        {
          userFromA {
            __typename
            id
            profile { __typename displayName }
            ... on User @skip(if: $skip) {
              __typename
              profile {
                __typename
                displayName
                ... on GuestAccount { guestToken accountType }
                ... on AdminAccount { adminLevel accountType }
              }
              id
            }
          }
        }
      vars
        $skip

    [8] b   after [2]
      alias _e0
        if
          !$skip
        merge
          userFromA
        in
          ... on User {
            __typename
            profile {
              displayName
              ... on AdminAccount { accountType adminLevel }
              ... on GuestAccount { accountType guestToken }
            }
            id
          }
        out
          ... on User { permissions }
      alias _e1
        merge
          userFromA
        in
          ... on User { __typename profile { displayName } id }
        out
          ... on User { basicPermissions }
    ");

    Ok(())
}

/// Same as above, plus `permissions` on another `userFromA` under `@include`. Neither
/// conditional call shares with `basicPermissions`, and the two conditional ones stay separate
/// too.
#[test]
fn merged_conditional_requires_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!, $include: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
          }
          userFromA @include(if: $include) {
            permissions
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-fragments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $include: Boolean!, $skip: Boolean!

    fetches
      fetch               after
      [2] a               -
      [12] b              [2]

    [2] a
      out
        {
          userFromA {
            __typename
            id
            profile { __typename displayName }
            ... on User @skip(if: $skip) { ...a }
          }
          userFromA @include(if: $include) { ...a }
        }
        fragment a on User {
          __typename
          profile {
            __typename
            displayName
            ... on GuestAccount { guestToken accountType }
            ... on AdminAccount { adminLevel accountType }
          }
          id
        }
      vars
        $include, $skip

    [12] b   after [2]
      alias _e0
        if
          $include
        merge
          userFromA
        in
          ... on User {
            __typename
            profile {
              displayName
              ... on AdminAccount { accountType adminLevel }
              ... on GuestAccount { accountType guestToken }
            }
            id
          }
        out
          ... on User { ...a }
      alias _e1
        if
          !$skip
        merge
          userFromA
        in
          ... on User {
            __typename
            profile {
              displayName
              ... on AdminAccount { accountType adminLevel }
              ... on GuestAccount { accountType guestToken }
            }
            id
          }
        out
          ... on User { ...a }
      alias _e2
        merge
          userFromA
        in
          ... on User { __typename profile { displayName } id }
        out
          ... on User { basicPermissions }
      fragments
        fragment a on User { permissions }
    ");

    Ok(())
}

#[test]
fn merged_conditional_inputs_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!, $include: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
            other: permissions @include(if: $include)
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-fragments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $include: Boolean!, $skip: Boolean!

    fetches
      fetch               after
      [2] a               -
      [12] b              [2]

    [2] a
      out
        {
          userFromA {
            __typename
            id
            profile { __typename displayName }
            ... on User @skip(if: $skip) { ...a }
            ... on User @include(if: $include) { ...a }
          }
        }
        fragment a on User {
          __typename
          profile {
            __typename
            displayName
            ... on GuestAccount { guestToken accountType }
            ... on AdminAccount { adminLevel accountType }
          }
          id
        }
      vars
        $include, $skip

    [12] b   after [2]
      alias _e0
        if
          !$skip || $include
        merge
          userFromA
        in
          ... on User {
            __typename
            profile {
              displayName
              ... on AdminAccount { accountType adminLevel }
              ... on GuestAccount { accountType guestToken }
            }
            id
          }
        out
          ... on User {
            ... on User @include(if: $include) { other: permissions }
            ... on User @skip(if: $skip) { permissions }
          }
      alias _e1
        merge
          userFromA
        in
          ... on User { __typename profile { displayName } id }
        out
          ... on User { basicPermissions }
      vars
        $include, $skip
    ");

    Ok(())
}
