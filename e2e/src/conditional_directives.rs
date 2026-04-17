#[cfg(test)]
mod conditional_directives_e2e_tests {
    // These tests ensure the behavior when a selection has both @skip and @include directives.
    // The expected behavior is;
    // 1. If @skip(if: $skip) is true, the selection should be skipped regardless of the @include directive.
    // 2. If @skip(if: $skip) is false, the selection should be included only if @include(if: $include) is true.
    // 3. If both @skip(if: $skip) and @include(if: $include) are false, the selection should be skipped.
    // 4. If both @skip(if: $skip) and @include(if: $include) are true, the selection should be skipped.

    use sonic_rs::{pointer, JsonValueTrait};

    use crate::testkit::{ClientResponseExt, Started, TestRouter, TestSubgraphs};

    async fn build_router_with_supergraph() -> (TestSubgraphs<Started>, TestRouter<Started>) {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;
        (subgraphs, router)
    }

    fn check_response_includes_product_name(json_body: sonic_rs::Value, expected_included: bool) {
        let product_name =
            json_body.pointer(&pointer!["data", "me", "reviews", 0, "product", "name",]);
        if expected_included {
            assert!(
                product_name.is_some(),
                "Expected product name to be included, but it was not. Response body: {}",
                json_body
            );
        } else {
            assert!(
                product_name.is_none(),
                "Expected product name to be skipped, but it was included. Response body: {}",
                json_body
            );
        }
    }

    async fn run_test(query: &str, skip: bool, include: bool, expected_included: bool) {
        let (_subgraphs, router) = build_router_with_supergraph().await;
        let variables = sonic_rs::json!({
            "include": include,
            "skip": skip,
        });
        let res = router
            .send_graphql_request(query, Some(variables), None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;
        let data = json_body.pointer(&pointer!["data"]);
        assert!(
            data.is_some_and(|value| value.is_object()),
            "Expected response.data to be an object. Response body: {}",
            json_body
        );
        let errors = json_body.pointer(&pointer!["errors"]);
        assert!(
            errors.is_none_or(|value| value.is_null()),
            "Expected response.errors to be null or missing. Response body: {}",
            json_body
        );

        check_response_includes_product_name(json_body, expected_included);
    }

    const FIELD_CONDITIONS_SKIP_THEN_INCLUDE_QUERY: &'static str = r#"
        query($skip: Boolean!, $include: Boolean!) {
            me {
                name
                reviews {
                    product {
                        upc 
                        name @skip(if: $skip) @include(if: $include)
                    }
                }
            }
        }
    "#;

    const FIELD_CONDITIONS_INCLUDE_THEN_SKIP_QUERY: &'static str = r#"
        query($skip: Boolean!, $include: Boolean!) {
            me {
                name
                reviews {
                    product {
                        upc 
                        name @include(if: $include) @skip(if: $skip)
                    }
                }
            }
        }
    "#;

    const INLINE_FRAGMENT_CONDITIONS_SKIP_THEN_INCLUDE_QUERY: &'static str = r#"
        query($skip: Boolean!, $include: Boolean!) {
            me {
                name
                reviews {
                    product {
                        upc 
                        ... on Product @skip(if: $skip) @include(if: $include) {
                            name
                        }
                    }
                }
            }
        }
    "#;

    const INLINE_FRAGMENT_CONDITIONS_INCLUDE_THEN_SKIP_QUERY: &'static str = r#"
        query($skip: Boolean!, $include: Boolean!) {
            me {
                name
                reviews {
                    product {
                        upc 
                        ... on Product @include(if: $include) @skip(if: $skip) {
                            name
                        }
                    }
                }
            }
        }
    "#;

    // If skip: true, the selection should be skipped regardless of the include value
    #[ntex::test]
    async fn field_skip_true_and_include_true() {
        run_test(FIELD_CONDITIONS_SKIP_THEN_INCLUDE_QUERY, true, true, false).await;
    }
    #[ntex::test]
    async fn field_skip_true_and_include_false() {
        run_test(FIELD_CONDITIONS_SKIP_THEN_INCLUDE_QUERY, true, false, false).await;
    }

    // If skip: false, the selection should be included only if include: true
    #[ntex::test]
    async fn field_skip_false_and_include_true() {
        run_test(FIELD_CONDITIONS_SKIP_THEN_INCLUDE_QUERY, false, true, true).await;
    }
    #[ntex::test]
    async fn field_skip_false_and_include_false() {
        run_test(
            FIELD_CONDITIONS_SKIP_THEN_INCLUDE_QUERY,
            false,
            false,
            false,
        )
        .await;
    }

    // Make sure the order of directives does not matter
    // So this time, `@include` comes before `@skip`, but the behavior should be the same as the previous 4 tests
    #[ntex::test]
    async fn field_include_then_skip_skip_true_and_include_true() {
        // In this case, `@skip` is true, so the selection should be skipped regardless of the `@include` value
        run_test(FIELD_CONDITIONS_INCLUDE_THEN_SKIP_QUERY, true, true, false).await;
    }
    #[ntex::test]
    async fn field_include_then_skip_skip_true_and_include_false() {
        // In this case, `@skip` is true, so the selection should be skipped regardless of the `@include` value
        run_test(FIELD_CONDITIONS_INCLUDE_THEN_SKIP_QUERY, true, false, false).await;
    }
    #[ntex::test]
    async fn field_include_then_skip_skip_false_and_include_true() {
        // In this case, `@skip` is false and `@include` is true, so the selection should be included
        run_test(FIELD_CONDITIONS_INCLUDE_THEN_SKIP_QUERY, false, true, true).await;
    }
    #[ntex::test]
    async fn field_include_then_skip_skip_false_and_include_false() {
        // In this case, `@skip` is false but `@include` is also false, so the selection should be skipped
        run_test(
            FIELD_CONDITIONS_INCLUDE_THEN_SKIP_QUERY,
            false,
            false,
            false,
        )
        .await;
    }

    // If skip: true, the selection should be skipped regardless of the include value
    #[ntex::test]
    async fn inline_fragment_skip_true_and_include_true() {
        // In this case, `@skip` is true, so the selection should be skipped regardless of the `@include` value
        run_test(
            INLINE_FRAGMENT_CONDITIONS_SKIP_THEN_INCLUDE_QUERY,
            true,
            true,
            false,
        )
        .await;
    }
    #[ntex::test]
    async fn inline_fragment_skip_true_and_include_false() {
        // In this case, `@skip` is true, so the selection should be skipped regardless of the `@include` value
        run_test(
            INLINE_FRAGMENT_CONDITIONS_SKIP_THEN_INCLUDE_QUERY,
            true,
            false,
            false,
        )
        .await;
    }
    // If skip: false, the selection should be included only if include: true
    #[ntex::test]
    async fn inline_fragment_skip_false_and_include_true() {
        // In this case, `@skip` is false and `@include` is true, so the selection should be included
        run_test(
            INLINE_FRAGMENT_CONDITIONS_SKIP_THEN_INCLUDE_QUERY,
            false,
            true,
            true,
        )
        .await;
    }
    #[ntex::test]
    async fn inline_fragment_skip_false_and_include_false() {
        // In this case, `@skip` is false but `@include` is also false, so the selection should be skipped
        run_test(
            INLINE_FRAGMENT_CONDITIONS_SKIP_THEN_INCLUDE_QUERY,
            false,
            false,
            false,
        )
        .await;
    }

    // Make sure the order of directives does not matter
    // So this time, `@include` comes before `@skip`, but the behavior should be the same as the previous 4 tests
    #[ntex::test]
    async fn inline_fragment_include_then_skip_skip_true_and_include_true() {
        run_test(
            INLINE_FRAGMENT_CONDITIONS_INCLUDE_THEN_SKIP_QUERY,
            true,
            true,
            false,
        )
        .await;
    }
    #[ntex::test]
    async fn inline_fragment_include_then_skip_skip_true_and_include_false() {
        run_test(
            INLINE_FRAGMENT_CONDITIONS_INCLUDE_THEN_SKIP_QUERY,
            true,
            false,
            false,
        )
        .await;
    }
    #[ntex::test]
    async fn inline_fragment_include_then_skip_skip_false_and_include_true() {
        run_test(
            INLINE_FRAGMENT_CONDITIONS_INCLUDE_THEN_SKIP_QUERY,
            false,
            true,
            true,
        )
        .await;
    }
    #[ntex::test]
    async fn inline_fragment_include_then_skip_skip_false_and_include_false() {
        run_test(
            INLINE_FRAGMENT_CONDITIONS_INCLUDE_THEN_SKIP_QUERY,
            false,
            false,
            false,
        )
        .await;
    }
}
