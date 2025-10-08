use http::{header, StatusCode};
use ntex::{
    http::{header::HeaderValue, HeaderMap},
    web::{self, HttpRequest},
};
use regex::Regex;

pub struct CORSPlan {
    allow_any_origin: bool,
    single_origin: Option<String>,
    origins: Option<Vec<String>>,
    match_origin: Option<Vec<Regex>>,
    methods_value: Option<HeaderValue>,
    allow_headers_value: Option<HeaderValue>,
    expose_headers_value: Option<HeaderValue>,
    allow_credentials_value: Option<HeaderValue>,
    max_age_value: Option<HeaderValue>,
}

impl CORSPlan {
    pub fn from_config(config: &hive_router_config::cors::CORSConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }

        let allow_credentials_value = if config.allow_credentials {
            Some(HeaderValue::from_static("true"))
        } else {
            None
        };

        let max_age_value = if let Some(max_age) = config.max_age {
            let max_age_str = max_age.to_string();
            HeaderValue::from_str(&max_age_str).ok()
        } else {
            None
        };

        let match_origin = config.match_origin.as_ref().map(|patterns| {
            patterns
                .iter()
                .filter_map(|pattern| Regex::new(pattern).ok())
                .collect()
        });

        let methods_value = create_header_value_from_vec_str(&config.methods);

        let allow_headers_value = create_header_value_from_vec_str(&config.allow_headers);

        let expose_headers_value = create_header_value_from_vec_str(&config.expose_headers);

        let single_origin: Option<String>;
        let origins: Option<Vec<String>>;
        if let Some(origins_config) = &config.origins {
            if origins_config.len() == 1 {
                single_origin = Some(origins_config[0].clone());
                origins = None;
            } else {
                single_origin = None;
                origins = Some(origins_config.clone());
            }
        } else {
            single_origin = None;
            origins = None;
        }

        Some(Self {
            allow_any_origin: config.allow_any_origin,
            single_origin,
            origins,
            match_origin,
            expose_headers_value,
            allow_credentials_value,
            max_age_value,
            methods_value,
            allow_headers_value,
        })
    }
}

fn create_header_value_from_vec_str(vec: &Option<Vec<String>>) -> Option<HeaderValue> {
    if let Some(vec) = vec {
        if vec.is_empty() {
            return None;
        }
        let joined = vec.join(", ");
        HeaderValue::from_str(&joined).ok()
    } else {
        None
    }
}

pub enum CORSResult {
    EarlyResponse(web::HttpResponse),
    Continue(HeaderMap),
    None,
}

pub fn perform_cors_on_request(req: &HttpRequest, cors: &CORSPlan) -> CORSResult {
    let mut headers = HeaderMap::new();
    if let Some(current_origin) = req.headers().get(header::ORIGIN) {
        if cors.allow_any_origin {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, current_origin.clone());
            headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        } else if let Some(single_origin) = &cors.single_origin {
            if let Ok(single_origin) = HeaderValue::from_str(single_origin) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, single_origin);
            }
        } else if cors
            .origins
            .as_ref()
            .is_some_and(|origins| origins.iter().any(|o| current_origin == o.as_str()))
        {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, current_origin.clone());
            headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        } else if let Ok(current_origin_str) = current_origin.to_str() {
            if cors
                .match_origin
                .as_ref()
                .is_some_and(|patterns| patterns.iter().any(|re| re.is_match(current_origin_str)))
            {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, current_origin.clone());
                headers.insert(header::VARY, HeaderValue::from_static("Origin"));
            } else {
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    HeaderValue::from_static("null"),
                );
            }
        }

        if let Some(methods_value) = &cors.methods_value {
            headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, methods_value.clone());
        } else if let Some(request_method) =
            req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD)
        {
            headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, request_method.clone());
        }

        if let Some(allow_headers_value) = &cors.allow_headers_value {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                allow_headers_value.clone(),
            );
        } else if let Some(request_headers) =
            req.headers().get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                request_headers.clone(),
            );
            if let Some(existing_vary) = headers.get(header::VARY).and_then(|v| v.to_str().ok()) {
                let new_vary = if existing_vary.is_empty() {
                    "Access-Control-Request-Headers".to_string()
                } else {
                    format!("{}, Access-Control-Request-Headers", existing_vary)
                };
                if let Ok(new_vary) = HeaderValue::from_str(&new_vary) {
                    headers.insert(header::VARY, new_vary);
                }
            } else {
                headers.insert(
                    header::VARY,
                    HeaderValue::from_static("Access-Control-Request-Headers"),
                );
            }
        }

        if let Some(allow_credentials_value) = &cors.allow_credentials_value {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                allow_credentials_value.clone(),
            );
        }

        if let Some(expose_headers_value) = &cors.expose_headers_value {
            headers.insert(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                expose_headers_value.clone(),
            );
        }

        if let Some(max_age_value) = &cors.max_age_value {
            headers.insert(header::ACCESS_CONTROL_MAX_AGE, max_age_value.clone());
        }
    }

    if req.method() == ntex::http::Method::OPTIONS {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
        let mut response = web::HttpResponse::Ok()
            .status(StatusCode::NO_CONTENT)
            .finish();

        *response.headers_mut() = headers;

        CORSResult::EarlyResponse(response)
    } else if headers.is_empty() {
        CORSResult::None
    } else {
        CORSResult::Continue(headers)
    }
}

#[cfg(test)]
mod tests {
    use ntex::{http::header, web::test::TestRequest};

    use crate::pipeline::cors::{perform_cors_on_request, CORSPlan, CORSResult};

    #[test]
    fn options_call_responds_with_correct_status_and_headers() {
        let cors_config = hive_router_config::cors::CORSConfig {
            enabled: true,
            allow_any_origin: true,
            ..hive_router_config::cors::CORSConfig::default()
        };
        let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
        let req = TestRequest::with_uri("/graphql")
            .method(ntex::http::Method::OPTIONS)
            .to_http_request();
        if let CORSResult::EarlyResponse(res) = perform_cors_on_request(&req, &cors_plan) {
            assert_eq!(res.status(), ntex::http::StatusCode::NO_CONTENT);
            assert_eq!(res.headers().get(header::CONTENT_LENGTH).unwrap(), "0");
        } else {
            panic!("Expected EarlyResponse variant");
        }
    }

    mod no_origin_specified {
        use ntex::{http::header, web::test::TestRequest};

        use crate::pipeline::cors::{perform_cors_on_request, CORSPlan, CORSResult};

        #[test]
        fn no_cors_headers_if_no_origin_present_on_the_request_headers() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .to_http_request();
            assert!(matches!(
                perform_cors_on_request(&req, &cors_plan),
                CORSResult::None
            ));
        }

        #[test]
        fn returns_the_origin_if_sent_with_request_headers() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            if let CORSResult::Continue(cors_headers) = perform_cors_on_request(&req, &cors_plan) {
                assert_eq!(
                    cors_headers.get("access-control-allow-origin").unwrap(),
                    "https://example.com"
                );
            } else {
                panic!("Expected Continue variant");
            }
        }
    }

    mod single_origin {
        use ntex::http::header;

        use crate::pipeline::cors::{perform_cors_on_request, CORSPlan, CORSResult};

        #[test]
        fn returns_the_origin_even_if_it_is_different_than_the_sent_origin() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                origins: Some(vec!["https://allowed.com".to_string()]),
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            if let CORSResult::Continue(cors_headers) = perform_cors_on_request(&req, &cors_plan) {
                assert_eq!(
                    cors_headers.get("access-control-allow-origin").unwrap(),
                    "https://allowed.com"
                );
            } else {
                panic!("Expected Continue variant");
            }
        }
    }

    mod multiple_origins {
        use ntex::http::header;

        use crate::pipeline::cors::{perform_cors_on_request, CORSPlan, CORSResult};

        #[test]
        fn returns_the_origin_itself_if_it_matches() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                origins: Some(vec![
                    "https://example.com".to_string(),
                    "https://another.com".to_string(),
                ]),
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let cors_response = perform_cors_on_request(&req, &cors_plan);
            if let CORSResult::Continue(cors_headers) = cors_response {
                assert_eq!(
                    cors_headers.get("access-control-allow-origin").unwrap(),
                    "https://example.com"
                );
            } else {
                panic!("Expected Continue variant");
            }
        }

        #[test]
        fn returns_null_if_it_does_not_match() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                origins: Some(vec![
                    "https://example.com".to_string(),
                    "https://another.com".to_string(),
                ]),
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://notallowed.com")
                .to_http_request();
            if let CORSResult::Continue(cors_headers) = perform_cors_on_request(&req, &cors_plan) {
                assert_eq!(
                    cors_headers.get("access-control-allow-origin").unwrap(),
                    "null"
                );
            } else {
                panic!("Expected Continue variant");
            }
        }
    }

    mod vary_header {
        use ntex::http::header;

        use crate::pipeline::cors::{perform_cors_on_request, CORSPlan, CORSResult};

        #[test]
        fn returns_vary_with_multiple_values() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                origins: Some(vec![
                    "https://example.com".to_string(),
                    "https://another.com".to_string(),
                ]),
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "X-Custom-Header")
                .to_http_request();
            if let CORSResult::Continue(cors_headers) = perform_cors_on_request(&req, &cors_plan) {
                assert_eq!(
                    cors_headers.get("access-control-allow-origin").unwrap(),
                    "https://example.com"
                );
                assert_eq!(
                    cors_headers.get("vary").unwrap(),
                    "Origin, Access-Control-Request-Headers"
                );
            } else {
                panic!("Expected Continue variant");
            }
        }
    }
}
