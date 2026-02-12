pub mod compile;
pub mod errors;
pub mod expression;
pub mod plan;
pub mod request;
pub mod response;
pub mod sanitizer;

#[cfg(test)]
mod tests {
    use crate::{
        execution::client_request_details::{
            ClientRequestDetails, JwtRequestDetails, OperationDetails,
        },
        headers::{
            compile::compile_headers_plan,
            plan::ResponseHeaderAggregator,
            request::modify_subgraph_request_headers,
            response::{apply_subgraph_response_headers},
        },
    };
    use hive_router_config::parse_yaml_config;
    use http::{HeaderMap, HeaderName, HeaderValue};
    use ntex::http::HeaderMap as NtexHeaderMap;

    fn header_name_owned(s: &str) -> HeaderName {
        HeaderName::from_bytes(s.as_bytes()).unwrap()
    }
    fn header_value_owned(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap()
    }

    trait HeaderMapAsStringExt {
        fn to_string(&self) -> String;
    }

    impl HeaderMapAsStringExt for HeaderMap {
        fn to_string(&self) -> String {
            let mut buffer = String::new();

            for (name, value) in self.iter() {
                buffer.push_str(&format!(
                    "{}: {}\n",
                    name.as_str(),
                    value.to_str().unwrap_or("<invalid utf8>")
                ));
            }

            buffer
        }
    }

    impl HeaderMapAsStringExt for ntex::http::HeaderMap {
        fn to_string(&self) -> String {
            let mut buffer = String::new();

            for (name, value) in self.iter() {
                buffer.push_str(&format!(
                    "{}: {}\n",
                    name.as_str(),
                    value.to_str().unwrap_or("<invalid utf8>")
                ));
            }

            buffer
        }
    }

    #[test]
    fn test_build_subgraph_headers_propagate_and_set() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - propagate:
                    named: x-prop
                    rename: x-renamed
                - insert:
                    name: x-set
                    value: set-value
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();

        let plan = compile_headers_plan(&config.headers).unwrap();

        let mut client_headers = NtexHeaderMap::new();
        client_headers.insert(
            header_name_owned("x-prop"),
            header_value_owned("abc").into(),
        );

        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        insta::assert_snapshot!(out.to_string(), @r#"
          x-renamed: abc
          x-set: set-value
        "#);
    }

    #[test]
    fn test_build_subgraph_headers_with_default() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - propagate:
                    named: x-missing
                    default: default-value
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };
        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        insta::assert_snapshot!(out.to_string(), @r#"
          x-missing: default-value
        "#);
    }

    // Tests that `matching` and `exclude` rules are correctly applied for propagation.
    #[test]
    fn test_propagate_with_matching_and_exclude() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - propagate:
                    matching: "^x-.*"
                    exclude: ["^x-secret-.*"]
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();

        let mut client_headers = NtexHeaderMap::new();
        client_headers.insert(
            header_name_owned("x-forward-this"),
            header_value_owned("value1").into(),
        );
        client_headers.insert(
            header_name_owned("x-secret-header"),
            header_value_owned("value2").into(),
        );
        client_headers.insert(
            header_name_owned("authorization"),
            header_value_owned("value3").into(),
        );

        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        assert_eq!(out.get("x-forward-this").unwrap(), "value1");
        assert!(out.get("x-secret-header").is_none());
        assert!(out.get("authorization").is_none());

        insta::assert_snapshot!(out.to_string(), @r#"
          x-forward-this: value1
        "#);
    }

    // Tests inserting a header with a value from a VRL expression.
    #[test]
    fn test_insert_request_header_with_expression() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - insert:
                    name: x-operation-name
                    expression: '.request.operation.name || "unknown"'
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: Some("MyQuery"),
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        insta::assert_snapshot!(out.to_string(), @r#"
          x-operation-name: MyQuery
        "#);
    }

    // Tests VRL expression fallback to a default value when a field is null.
    #[test]
    fn test_insert_request_header_with_expression_fallback() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - insert:
                    name: x-operation-name
                    expression: '.request.operation.name || "unknown"'
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        insta::assert_snapshot!(out.to_string(), @r#"
          x-operation-name: unknown
        "#);
    }

    // Tests that subgraph-specific rules override global `all` rules.
    #[test]
    fn test_subgraph_specific_request_rules() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - insert:
                    name: x-scope
                    value: all
            subgraphs:
              accounts:
                request:
                  - insert:
                      name: x-scope
                      value: accounts
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        // For "accounts" subgraph, the specific rule should apply.
        let mut out_accounts = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "accounts", &client_details, &mut out_accounts)
            .unwrap();

        insta::assert_snapshot!(out_accounts.to_string(), @r#"
          x-scope: accounts
        "#);

        // For any other subgraph, the `all` rule should apply.
        let mut out_other = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "products", &client_details, &mut out_other)
            .unwrap();

        insta::assert_snapshot!(out_other.to_string(), @r#"
          x-scope: all
        "#);
    }

    #[test]
    fn test_apply_subgraph_response_headers_and_finalize() {
        let yaml_str = r#"
          headers:
            all:
              response:
                - propagate:
                    named: x-resp
                    algorithm: last
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut accumulator = ResponseHeaderAggregator::default();

        let mut subgraph_headers = HeaderMap::new();
        subgraph_headers.insert(
            header_name_owned("x-resp"),
            header_value_owned("resp-value-1"),
        );
        apply_subgraph_response_headers(
            &plan,
            "any",
            &subgraph_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut subgraph_headers = HeaderMap::new();
        subgraph_headers.insert(
            header_name_owned("x-resp"),
            header_value_owned("resp-value-2"),
        );

        apply_subgraph_response_headers(
            &plan,
            "any",
            &subgraph_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut response_builder = ntex::http::Response::Ok();
        accumulator
            .modify_client_response_headers(&mut response_builder)
            .unwrap();
        let response = response_builder.finish();
        let final_headers = response.headers();

        insta::assert_snapshot!(final_headers.to_string(), @r#"
          x-resp: resp-value-2
        "#);
    }

    // Tests the `first` algorithm for response header propagation.
    #[test]
    fn test_response_propagate_first() {
        let yaml_str = r#"
          headers:
            all:
              response:
                - propagate:
                    named: x-resp
                    algorithm: first
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut accumulator = ResponseHeaderAggregator::default();

        let mut subgraph_headers_1 = HeaderMap::new();
        subgraph_headers_1.insert(
            header_name_owned("x-resp"),
            header_value_owned("resp-value-1"),
        );
        apply_subgraph_response_headers(
            &plan,
            "any",
            &subgraph_headers_1,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut subgraph_headers_2 = HeaderMap::new();
        subgraph_headers_2.insert(
            header_name_owned("x-resp"),
            header_value_owned("resp-value-2"),
        );
        apply_subgraph_response_headers(
            &plan,
            "any",
            &subgraph_headers_2,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut response_builder = ntex::http::Response::Ok();
        accumulator
            .modify_client_response_headers(&mut response_builder)
            .unwrap();
        let response = response_builder.finish();
        let final_headers = response.headers();

        insta::assert_snapshot!(final_headers.to_string(), @r#"
          x-resp: resp-value-1
        "#);
    }

    // Tests the `append` algorithm for response header propagation.
    #[test]
    fn test_response_propagate_append() {
        let yaml_str = r#"
          headers:
            all:
              response:
                - propagate:
                    named: x-stuff
                    algorithm: append
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };
        let mut accumulator = ResponseHeaderAggregator::default();

        let mut subgraph1_headers = HeaderMap::new();
        subgraph1_headers.insert(header_name_owned("x-stuff"), header_value_owned("val1"));
        apply_subgraph_response_headers(
            &plan,
            "subgraph1",
            &subgraph1_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut subgraph2_headers = HeaderMap::new();
        subgraph2_headers.insert(header_name_owned("x-stuff"), header_value_owned("val2"));
        apply_subgraph_response_headers(
            &plan,
            "subgraph2",
            &subgraph2_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut response_builder = ntex::http::Response::Ok();
        accumulator
            .modify_client_response_headers(&mut response_builder)
            .unwrap();
        let response = response_builder.finish();
        let final_headers = response.headers();

        insta::assert_snapshot!(final_headers.to_string(), @r#"
          x-stuff: val1, val2
        "#);
    }

    // Tests that "never-join" headers like set-cookie are appended as separate fields.
    #[test]
    fn test_response_propagate_append_never_join() {
        let yaml_str = r#"
          headers:
            all:
              response:
                - propagate:
                    named: set-cookie
                    algorithm: append
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };
        let mut accumulator = ResponseHeaderAggregator::default();

        let mut subgraph1_headers = HeaderMap::new();
        subgraph1_headers.insert(header_name_owned("set-cookie"), header_value_owned("a=1"));
        apply_subgraph_response_headers(
            &plan,
            "subgraph1",
            &subgraph1_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut subgraph2_headers = HeaderMap::new();
        subgraph2_headers.insert(header_name_owned("set-cookie"), header_value_owned("b=2"));
        apply_subgraph_response_headers(
            &plan,
            "subgraph2",
            &subgraph2_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut response_builder = ntex::http::Response::Ok();
        accumulator
            .modify_client_response_headers(&mut response_builder)
            .unwrap();
        let response = response_builder.finish();
        let final_headers = response.headers();

        insta::assert_snapshot!(final_headers.to_string(), @r#"
          set-cookie: a=1
          set-cookie: b=2
        "#);
    }

    // Tests inserting a response header with a value from a VRL expression.
    #[test]
    fn test_insert_response_header_with_expression() {
        let yaml_str = r#"
          headers:
            all:
              response:
                - insert:
                    name: x-original-forwarded-for
                    expression: '.response."x-forwarded-for"'
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();
        let client_headers = NtexHeaderMap::new();
        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut accumulator = ResponseHeaderAggregator::default();

        let mut subgraph_headers = HeaderMap::new();
        subgraph_headers.insert(
            header_name_owned("x-forwarded-for"),
            header_value_owned("1.2.3.4"),
        );

        apply_subgraph_response_headers(
            &plan,
            "any",
            &subgraph_headers,
            &client_details,
            &mut accumulator,
        )
        .unwrap();

        let mut response_builder = ntex::http::Response::Ok();
        accumulator
            .modify_client_response_headers(&mut response_builder)
            .unwrap();
        let response = response_builder.finish();
        let final_headers = response.headers();

        insta::assert_snapshot!(final_headers.to_string(), @r#"
          x-original-forwarded-for: 1.2.3.4
        "#);
    }

    #[test]
    fn test_remove_header() {
        let yaml_str = r#"
          headers:
            all:
              request:
                - propagate:
                    named: x-keep
                - remove:
                    named: x-remove
        "#;
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let plan = compile_headers_plan(&config.headers).unwrap();

        let mut client_headers = NtexHeaderMap::new();

        client_headers.insert(
            header_name_owned("x-remove"),
            header_value_owned("bye").into(),
        );
        client_headers.insert(header_name_owned("x-keep"), header_value_owned("hi").into());

        let client_details = ClientRequestDetails {
            method: &http::Method::POST,
            url: &"http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
            jwt: JwtRequestDetails::Unauthenticated,
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out).unwrap();

        insta::assert_snapshot!(out.to_string(), @r#"
          x-keep: hi
        "#);
    }
}
