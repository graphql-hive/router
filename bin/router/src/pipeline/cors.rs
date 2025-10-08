use hive_router_config::cors::CORSPolicyConfig;
use http::{header, StatusCode};
use ntex::{
    http::{header::HeaderValue, HeaderMap},
    web::{self, HttpRequest},
};
use regex::Regex;

pub struct CORSPlanPolicy {
    methods_value: Option<HeaderValue>,
    allow_headers_value: Option<HeaderValue>,
    expose_headers_value: Option<HeaderValue>,
    allow_credentials_value: Option<HeaderValue>,
    max_age_value: Option<HeaderValue>,
}

pub struct CORSPlanByOrigin {
    origins: Vec<String>,
    patterns: Vec<Regex>,
    policy: CORSPlanPolicy,
}

#[derive(thiserror::Error, Debug)]
pub enum CORSConfigError {
    #[error("invalid regex pattern in CORS config: {0}, {1}")]
    InvalidRegex(String, String),
}

impl CORSPlanByOrigin {
    pub fn try_from_config(
        config: &CORSPolicyConfig,
        global_policy: &CORSPlanPolicy,
    ) -> Result<Self, CORSConfigError> {
        let policy = CORSPlanPolicy::from_config(config, global_policy);
        let patterns = config
            .match_origin
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|pattern| {
                Regex::new(pattern).map_err(|err| {
                    CORSConfigError::InvalidRegex(pattern.to_string(), err.to_string())
                })
            })
            .collect::<Result<Vec<Regex>, CORSConfigError>>()?;
        Ok(CORSPlanByOrigin {
            origins: config.origins.clone().unwrap_or_default(),
            patterns,
            policy,
        })
    }
    pub fn matches_origin(&self, origin: &str) -> bool {
        if self.origins.iter().any(|o| o == origin) {
            return true;
        }
        if self.patterns.iter().any(|re| re.is_match(origin)) {
            return true;
        }
        false
    }
}

pub enum CORSPlan {
    AllowAll {
        policy: CORSPlanPolicy,
    },
    Single {
        origin: String,
        policy: CORSPlanPolicy,
    },
    Multiple {
        plans: Vec<CORSPlanByOrigin>,
    },
}

impl CORSPlanPolicy {
    pub fn from_config(policy_config: &CORSPolicyConfig, global_policy: &CORSPlanPolicy) -> Self {
        CORSPlanPolicy {
            methods_value: create_header_value_from_vec_str(&policy_config.methods)
                .or_else(|| global_policy.methods_value.clone()),
            allow_headers_value: create_header_value_from_vec_str(&policy_config.allow_headers)
                .or_else(|| global_policy.allow_headers_value.clone()),
            expose_headers_value: create_header_value_from_vec_str(&policy_config.expose_headers)
                .or_else(|| global_policy.expose_headers_value.clone()),
            allow_credentials_value: if policy_config.allow_credentials == Some(true) {
                Some(HeaderValue::from_static("true"))
            } else {
                global_policy.allow_credentials_value.clone()
            },
            max_age_value: if let Some(max_age) = policy_config.max_age {
                let max_age_str = max_age.to_string();
                HeaderValue::from_str(&max_age_str).ok()
            } else {
                global_policy.max_age_value.clone()
            },
        }
    }
}

impl CORSPlan {
    pub fn from_config(
        config: &hive_router_config::cors::CORSConfig,
    ) -> Result<Option<Self>, CORSConfigError> {
        if !config.enabled {
            return Ok(None);
        }

        let global_policy = CORSPlanPolicy {
            methods_value: create_header_value_from_vec_str(&config.methods),
            allow_headers_value: create_header_value_from_vec_str(&config.allow_headers),
            expose_headers_value: create_header_value_from_vec_str(&config.expose_headers),
            allow_credentials_value: if config.allow_credentials {
                Some(HeaderValue::from_static("true"))
            } else {
                None
            },
            max_age_value: if let Some(max_age) = config.max_age {
                let max_age_str = max_age.to_string();
                HeaderValue::from_str(&max_age_str).ok()
            } else {
                None
            },
        };

        if config.allow_any_origin {
            Ok(Some(CORSPlan::AllowAll {
                policy: global_policy,
            }))
        } else if let [policy_config] = config.policies.as_slice() {
            let plan_policy = CORSPlanPolicy::from_config(policy_config, &global_policy);
            if policy_config.match_origin.is_none() {
                if let Some([origin]) = policy_config.origins.as_deref() {
                    return Ok(Some(CORSPlan::Single {
                        origin: origin.to_string(),
                        policy: plan_policy,
                    }));
                }
            }

            Ok(Some(CORSPlan::Multiple {
                plans: vec![CORSPlanByOrigin::try_from_config(
                    policy_config,
                    &global_policy,
                )?],
            }))
        } else if config.policies.len() > 1 {
            let plans = config
                .policies
                .iter()
                .map(|policy_config| {
                    CORSPlanByOrigin::try_from_config(policy_config, &global_policy)
                })
                .collect::<Result<Vec<CORSPlanByOrigin>, CORSConfigError>>()?;
            Ok(Some(CORSPlan::Multiple { plans }))
        } else {
            Ok(None)
        }
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

impl CORSPlan {
    pub fn get_early_response(&self, req: &HttpRequest) -> Option<web::HttpResponse> {
        if req.method() == ntex::http::Method::OPTIONS {
            let mut response = web::HttpResponse::Ok()
                .status(StatusCode::NO_CONTENT)
                .header(header::CONTENT_LENGTH, HeaderValue::from_static("0"))
                .finish();

            self.set_headers(req, response.headers_mut());

            Some(response)
        } else {
            None
        }
    }
    pub fn set_headers(&self, req: &HttpRequest, headers: &mut HeaderMap) {
        if let Some(current_origin) = req.headers().get(header::ORIGIN) {
            let policy = match self {
                CORSPlan::AllowAll { policy } => {
                    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, current_origin.clone());
                    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
                    Some(policy)
                }
                CORSPlan::Single { origin, policy } => {
                    if let Ok(single_origin) = HeaderValue::from_str(origin) {
                        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, single_origin);
                    }
                    Some(policy)
                }
                CORSPlan::Multiple { plans } => {
                    let current_origin_str = current_origin.to_str().ok().unwrap_or_default();
                    let matched_policy: Option<&CORSPlanPolicy> = plans.iter().find_map(|plan| {
                        if plan.matches_origin(current_origin_str) {
                            Some(&plan.policy)
                        } else {
                            None
                        }
                    });
                    if matched_policy.is_some() {
                        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, current_origin.clone());
                        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
                    } else {
                        headers.insert(
                            header::ACCESS_CONTROL_ALLOW_ORIGIN,
                            HeaderValue::from_static("null"),
                        );
                    }
                    matched_policy
                }
            };

            if let Some(policy) = policy {
                if let Some(methods_value) = &policy.methods_value {
                    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, methods_value.clone());
                } else if let Some(request_method) =
                    req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD)
                {
                    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, request_method.clone());
                }

                if let Some(allow_headers_value) = &policy.allow_headers_value {
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
                    if let Some(existing_vary) =
                        headers.get(header::VARY).and_then(|v| v.to_str().ok())
                    {
                        if !existing_vary.contains("Access-Control-Request-Headers") {
                            let new_vary = if existing_vary.is_empty() {
                                "Access-Control-Request-Headers".to_string()
                            } else {
                                format!("{}, Access-Control-Request-Headers", existing_vary)
                            };
                            if let Ok(new_vary) = HeaderValue::from_str(&new_vary) {
                                headers.insert(header::VARY, new_vary);
                            }
                        }
                    } else {
                        headers.insert(
                            header::VARY,
                            HeaderValue::from_static("Access-Control-Request-Headers"),
                        );
                    }
                }

                if let Some(allow_credentials_value) = &policy.allow_credentials_value {
                    headers.insert(
                        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                        allow_credentials_value.clone(),
                    );
                }

                if let Some(expose_headers_value) = &policy.expose_headers_value {
                    headers.insert(
                        header::ACCESS_CONTROL_EXPOSE_HEADERS,
                        expose_headers_value.clone(),
                    );
                }

                if let Some(max_age_value) = &policy.max_age_value {
                    headers.insert(header::ACCESS_CONTROL_MAX_AGE, max_age_value.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ntex::{http::header, web::test::TestRequest};

    use crate::pipeline::cors::CORSPlan;

    #[test]
    fn options_call_responds_with_correct_status_and_headers() {
        let cors_config = hive_router_config::cors::CORSConfig {
            enabled: true,
            allow_any_origin: true,
            ..hive_router_config::cors::CORSConfig::default()
        };
        let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
        let req = TestRequest::with_uri("/graphql")
            .method(ntex::http::Method::OPTIONS)
            .to_http_request();
        let early_response = cors_plan.get_early_response(&req);
        assert!(early_response.is_some());
        let res = early_response.unwrap();
        assert_eq!(res.status(), ntex::http::StatusCode::NO_CONTENT);
        assert_eq!(res.headers().get(header::CONTENT_LENGTH).unwrap(), "0");
    }

    mod no_origin_specified {
        use ntex::{http::header, web::test::TestRequest};

        use crate::pipeline::cors::CORSPlan;

        #[test]
        fn no_cors_headers_if_no_origin_present_on_the_request_headers() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert!(headers.len() == 0);
        }

        #[test]
        fn returns_the_origin_if_sent_with_request_headers() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert_eq!(
                headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://example.com"
            );
        }
    }

    mod single_origin {
        use ntex::http::header;

        use crate::pipeline::cors::CORSPlan;

        #[test]
        fn returns_the_origin_even_if_it_is_different_than_the_sent_origin() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![hive_router_config::cors::CORSPolicyConfig {
                    origins: Some(vec!["https://allowed.com".to_string()]),
                    ..Default::default()
                }],
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert_eq!(
                headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://allowed.com"
            );
        }
    }

    mod multiple_origins {
        use ntex::http::header;

        use crate::pipeline::cors::CORSPlan;

        #[test]
        fn returns_the_origin_itself_if_it_matches() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![hive_router_config::cors::CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert_eq!(
                headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://example.com"
            );
        }

        #[test]
        fn returns_null_if_it_does_not_match() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![hive_router_config::cors::CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://notallowed.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert_eq!(
                headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "null"
            );
        }
    }

    mod vary_header {
        use ntex::http::header;

        use crate::pipeline::cors::CORSPlan;

        #[test]
        fn returns_vary_with_multiple_values() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![hive_router_config::cors::CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "X-Custom-Header")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            cors_plan.set_headers(&req, &mut headers);
            assert_eq!(
                headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://example.com"
            );
            assert_eq!(
                headers.get("vary").unwrap(),
                "Origin, Access-Control-Request-Headers"
            );
        }
    }

    mod policies {
        use ntex::http::HeaderMap;

        use crate::pipeline::cors::CORSPlan;

        #[test]
        fn different_policies_for_different_origins() {
            let cors_config = hive_router_config::cors::CORSConfig {
                enabled: true,
                allow_any_origin: false,
                methods: Some(vec!["GET".to_string(), "POST".to_string()]),
                policies: vec![
                    hive_router_config::cors::CORSPolicyConfig {
                        origins: Some(vec!["https://example.com".to_string()]),
                        ..Default::default()
                    },
                    hive_router_config::cors::CORSPolicyConfig {
                        origins: Some(vec!["https://another.com".to_string()]),
                        methods: Some(vec!["GET".to_string()]),
                        ..Default::default()
                    },
                ],
                ..hive_router_config::cors::CORSConfig::default()
            };
            let cors_plan = CORSPlan::from_config(&cors_config).unwrap().unwrap();
            if let CORSPlan::Multiple { plans } = &cors_plan {
                assert_eq!(plans.len(), 2);
                assert_eq!(plans[0].origins, vec!["https://example.com"]);
                assert_eq!(plans[1].origins, vec!["https://another.com"]);
            } else {
                panic!("Expected ByOrigin variant");
            }

            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(ntex::http::header::CONTENT_TYPE, "application/json")
                .header(ntex::http::header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut cors_headers = HeaderMap::new();
            cors_plan.set_headers(&req, &mut cors_headers);
            assert_eq!(
                cors_headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://example.com"
            );
            assert_eq!(
                cors_headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_METHODS)
                    .unwrap(),
                "GET, POST"
            );

            let req = ntex::web::test::TestRequest::with_uri("/graphql")
                .method(ntex::http::Method::POST)
                .header(ntex::http::header::CONTENT_TYPE, "application/json")
                .header(ntex::http::header::ORIGIN, "https://another.com")
                .to_http_request();
            let mut cors_headers = HeaderMap::new();
            cors_plan.set_headers(&req, &mut cors_headers);

            assert_eq!(
                cors_headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://another.com"
            );
            assert_eq!(
                cors_headers
                    .get(ntex::http::header::ACCESS_CONTROL_ALLOW_METHODS)
                    .unwrap(),
                "GET"
            );
        }
    }
}
