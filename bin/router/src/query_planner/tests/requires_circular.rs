use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn requires_circular_1() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          feed {
            byNovice
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-circular.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [6] a               [5]
      [4] b               [6]

    [2] a
      out
        { feed { __typename id } }

    [5] b   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { author { __typename id } }

    [6] a   after [5]
      merge
        feed.@.author
      in
        ... on Author { __typename id }
      out
        ... on Author { yearsOfExperience }

    [4] b   after [6]
      merge
        feed.@
      in
        ... on Post { __typename author { yearsOfExperience } id }
      out
        ... on Post { byNovice }
    ");

    Ok(())
}

#[test]
fn requires_circular_2() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          feed {
            byExpert
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-circular.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [7] a               [6]
      [5] b               [7]
      [3] a               [5]

    [2] a
      out
        { feed { __typename id } }

    [6] b   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { author { __typename id } }

    [7] a   after [6]
      merge
        feed.@.author
      in
        ... on Author { __typename id }
      out
        ... on Author { yearsOfExperience }

    [5] b   after [7]
      merge
        feed.@
      in
        ... on Post { __typename author { yearsOfExperience } id }
      out
        ... on Post { byNovice }

    [3] a   after [5]
      merge
        feed.@
      in
        ... on Post { __typename byNovice id }
      out
        ... on Post { byExpert }
    ");

    Ok(())
}
