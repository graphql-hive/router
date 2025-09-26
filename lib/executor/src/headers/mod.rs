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
        execution::plan::{ClientRequestDetails, OperationDetails},
        headers::{
            compile::compile_headers_plan,
            plan::ResponseHeaderAggregator,
            request::modify_subgraph_request_headers,
            response::{apply_subgraph_response_headers, modify_client_response_headers},
        },
    };
    use hive_router_config::parse_yaml_config;
    use http::{HeaderMap, HeaderName, HeaderValue};
    use ntex_http::HeaderMap as NtexHeaderMap;

    fn header_name_owned(s: &str) -> HeaderName {
        HeaderName::from_bytes(s.as_bytes()).unwrap()
    }
    fn header_value_owned(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap()
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
            method: http::Method::POST,
            url: "http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out);

        assert_eq!(out.get("x-renamed").unwrap(), &header_value_owned("abc"));
        assert_eq!(out.get("x-set").unwrap(), &header_value_owned("set-value"));
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
            method: http::Method::POST,
            url: "http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
        };
        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out);

        assert_eq!(
            out.get("x-missing").unwrap(),
            &header_value_owned("default-value")
        );
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
            method: http::Method::POST,
            url: "http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
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
        );

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
        );

        let mut final_headers = HeaderMap::new();
        modify_client_response_headers(accumulator, &mut final_headers);

        assert_eq!(
            final_headers.get("x-resp").unwrap(),
            &header_value_owned("resp-value-2")
        );
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
            method: http::Method::POST,
            url: "http://example.com".parse().unwrap(),
            headers: &client_headers,
            operation: OperationDetails {
                name: None,
                query: "{ __typename }",
                kind: "query",
            },
        };

        let mut out = HeaderMap::new();
        modify_subgraph_request_headers(&plan, "any", &client_details, &mut out);

        assert!(out.get("x-remove").is_none());
        assert_eq!(out.get("x-keep").unwrap(), &header_value_owned("hi"));
    }
}
