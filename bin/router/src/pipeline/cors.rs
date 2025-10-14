use hive_router_config::cors::{CORSConfig, CORSPolicyConfig};
use http::{header, StatusCode};
use ntex::{
    http::{header::HeaderValue, HeaderMap},
    web::{self, HttpRequest},
};
// use regex::Regex;
use regex_automata::{
    meta::{BuildError, Regex},
    util::syntax::Config as SyntaxConfig,
};

#[derive(thiserror::Error, Debug)]
pub enum CORSConfigError {
    #[error("Failed to build regex for match_origin option. Please check your regex patterns for syntax errors. Reason: {0}")]
    InvalidRegex(#[from] Box<BuildError>),
}

pub struct CompiledCORSPolicy {
    methods_value: Option<HeaderValue>,
    allow_headers_value: Option<HeaderValue>,
    expose_headers_value: Option<HeaderValue>,
    allow_credentials_value: Option<HeaderValue>,
    max_age_value: Option<HeaderValue>,
}

impl CompiledCORSPolicy {
    pub fn from_config(policy_config: &CORSPolicyConfig, global: &CompiledCORSPolicy) -> Self {
        Self {
            methods_value: header_value_from_list(&policy_config.methods)
                .or_else(|| global.methods_value.clone()),
            allow_headers_value: header_value_from_list(&policy_config.allow_headers)
                .or_else(|| global.allow_headers_value.clone()),
            expose_headers_value: header_value_from_list(&policy_config.expose_headers)
                .or_else(|| global.expose_headers_value.clone()),
            allow_credentials_value: if policy_config.allow_credentials == Some(true) {
                Some(HeaderValue::from_static("true"))
            } else {
                global.allow_credentials_value.clone()
            },
            max_age_value: if let Some(max_age) = policy_config.max_age {
                HeaderValue::from_str(&max_age.to_string()).ok()
            } else {
                global.max_age_value.clone()
            },
        }
    }

    /// Apply this policy to the response headers, reflecting request hints as needed.
    /// `origin` should be the origin we want to send back (usually the request's
    /// `Origin` header) or `"null"` if unmatched.
    pub fn apply_to(
        &self,
        req: &HttpRequest,
        response_headers: &mut HeaderMap,
        origin: &HeaderValue,
    ) {
        // Access-Control-Allow-Origin + Vary: Origin
        response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        if origin.as_bytes() != b"null" {
            append_vary(response_headers, "Origin");
        }

        // Methods: prefer policy value; else reflect preflight request method when present
        if let Some(v) = &self.methods_value {
            response_headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, v.clone());
        } else if let Some(req_method) = req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD) {
            response_headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, req_method.clone());
        }

        // Allow-Headers: prefer policy value; else reflect preflight requested headers
        if let Some(v) = &self.allow_headers_value {
            response_headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, v.clone());
        } else if let Some(request_headers) =
            req.headers().get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        {
            response_headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                request_headers.clone(),
            );
            append_vary(response_headers, "Access-Control-Request-Headers");
        }

        if let Some(v) = &self.allow_credentials_value {
            response_headers.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, v.clone());
        }
        if let Some(v) = &self.expose_headers_value {
            response_headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, v.clone());
        }
        if let Some(v) = &self.max_age_value {
            response_headers.insert(header::ACCESS_CONTROL_MAX_AGE, v.clone());
        }
    }
}

pub struct CompiledOriginRule {
    pub origins: Vec<String>,
    pub pattern: Option<Regex>,
    pub policy: CompiledCORSPolicy,
}

fn build_regex_many(patterns: &[String]) -> Result<Option<Regex>, CORSConfigError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut regex_builder = Regex::builder();
    regex_builder.syntax(SyntaxConfig::new().unicode(false).utf8(false));
    regex_builder
        .build_many(patterns)
        .map(Some)
        .map_err(|e| Box::new(e).into())
}

impl CompiledOriginRule {
    pub fn try_from_config(
        config: &CORSPolicyConfig,
        global: &CompiledCORSPolicy,
    ) -> Result<Self, CORSConfigError> {
        let policy = CompiledCORSPolicy::from_config(config, global);
        let pattern = if let Some(patterns) = &config.match_origin {
            build_regex_many(patterns)?
        } else {
            None
        };

        Ok(Self {
            origins: config.origins.clone().unwrap_or_default(),
            pattern,
            policy,
        })
    }

    pub fn matches_origin(&self, origin: &str) -> bool {
        if self.origins.iter().any(|o| o == origin) {
            return true;
        }

        if self
            .pattern
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(origin))
        {
            return true;
        }

        false
    }
}

pub enum CORS {
    AllowAll { policy: CompiledCORSPolicy },
    ByOrigin { rules: Vec<CompiledOriginRule> },
}

impl CORS {
    pub fn from_config(config: &CORSConfig) -> Result<Option<Self>, CORSConfigError> {
        if !config.enabled {
            return Ok(None);
        }

        // Resolve global defaults
        let global = CompiledCORSPolicy {
            methods_value: header_value_from_list(&config.methods),
            allow_headers_value: header_value_from_list(&config.allow_headers),
            expose_headers_value: header_value_from_list(&config.expose_headers),
            allow_credentials_value: if config.allow_credentials {
                Some(HeaderValue::from_static("true"))
            } else {
                None
            },
            max_age_value: config
                .max_age
                .and_then(|v| HeaderValue::from_str(&v.to_string()).ok()),
        };

        if config.allow_any_origin {
            return Ok(Some(CORS::AllowAll { policy: global }));
        }

        // Resolve all origin rules
        let mut rules = Vec::with_capacity(config.policies.len());
        for policy in &config.policies {
            rules.push(CompiledOriginRule::try_from_config(policy, &global)?);
        }

        if rules.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CORS::ByOrigin { rules }))
        }
    }

    fn find_policy_for_origin(&self, origin: &str) -> Option<&CompiledCORSPolicy> {
        match self {
            CORS::AllowAll { policy } => Some(policy),
            CORS::ByOrigin { rules } => rules
                .iter()
                .find(|r| r.matches_origin(origin))
                .map(|r| &r.policy),
        }
    }

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
        let Some(current_origin) = req.headers().get(header::ORIGIN) else {
            return;
        };

        let origin_str = current_origin.to_str().ok().unwrap_or_default();
        if let Some(policy) = self.find_policy_for_origin(origin_str) {
            policy.apply_to(req, headers, current_origin);
        } else {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("null"),
            );
        }
    }
}

fn header_value_from_list(vec: &Option<Vec<String>>) -> Option<HeaderValue> {
    match vec.as_deref() {
        None | Some([]) => None,
        Some(v) => HeaderValue::from_str(&v.join(", ")).ok(),
    }
}

fn append_vary(headers: &mut HeaderMap, token: &str) {
    if let Some(existing) = headers.get(header::VARY).and_then(|v| v.to_str().ok()) {
        if existing
            .split(',')
            .map(|s| s.trim())
            .any(|t| t.eq_ignore_ascii_case(token))
        {
            // already present
            return;
        }

        let new_header_value = if existing.is_empty() {
            HeaderValue::from_str(token)
        } else {
            HeaderValue::from_str(&format!("{}, {}", existing, token))
        };

        if let Ok(v) = new_header_value {
            headers.insert(header::VARY, v);
        }

        return;
    }

    if let Ok(v) = HeaderValue::from_str(token) {
        headers.insert(header::VARY, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_router_config::cors::{CORSConfig, CORSPolicyConfig};
    use ntex::{
        http::header,
        http::{Method, StatusCode},
        web::test::TestRequest,
    };

    #[test]
    fn options_call_responds_with_correct_status_and_headers() {
        let cors_config = CORSConfig {
            enabled: true,
            allow_any_origin: true,
            ..CORSConfig::default()
        };
        let engine = CORS::from_config(&cors_config).unwrap().unwrap();
        let req = TestRequest::with_uri("/graphql")
            .method(Method::OPTIONS)
            .to_http_request();
        let early_response = engine.get_early_response(&req);
        assert!(early_response.is_some());
        let res = early_response.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        assert_eq!(res.headers().get(header::CONTENT_LENGTH).unwrap(), "0");
    }

    mod no_origin_specified {
        use super::*;

        #[test]
        fn no_cors_headers_if_no_origin_present_on_the_request_headers() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert!(headers.is_empty());
        }

        #[test]
        fn returns_the_origin_if_sent_with_request_headers() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: true,
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert_eq!(
                headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                "https://example.com"
            );
        }
    }

    mod single_origin_behavior {
        use super::*;

        #[test]
        fn returns_null_if_origin_does_not_match() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![CORSPolicyConfig {
                    origins: Some(vec!["https://allowed.com".to_string()]),
                    ..Default::default()
                }],
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert_eq!(
                headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                "null"
            );
        }
    }

    mod multiple_origins {
        use super::*;

        #[test]
        fn returns_the_origin_itself_if_it_matches() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert_eq!(
                headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                "https://example.com"
            );
        }

        #[test]
        fn returns_null_if_it_does_not_match() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://notallowed.com")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert_eq!(
                headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                "null"
            );
        }
    }

    mod vary_header {
        use super::*;

        #[test]
        fn returns_vary_with_multiple_values() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: false,
                policies: vec![CORSPolicyConfig {
                    origins: Some(vec![
                        "https://example.com".to_string(),
                        "https://another.com".to_string(),
                    ]),
                    ..Default::default()
                }],
                ..CORSConfig::default()
            };
            let engine = CORS::from_config(&cors_config).unwrap().unwrap();
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "X-Custom-Header")
                .to_http_request();
            let mut headers = header::HeaderMap::new();
            engine.set_headers(&req, &mut headers);
            assert_eq!(
                headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                "https://example.com"
            );
            assert_eq!(
                headers.get("vary").unwrap(),
                "Origin, Access-Control-Request-Headers"
            );
        }
    }

    mod policies {
        use super::*;

        #[test]
        fn different_policies_for_different_origins() {
            let cors_config = CORSConfig {
                enabled: true,
                allow_any_origin: false,
                methods: Some(vec!["GET".to_string(), "POST".to_string()]),
                policies: vec![
                    CORSPolicyConfig {
                        origins: Some(vec!["https://example.com".to_string()]),
                        ..Default::default()
                    },
                    CORSPolicyConfig {
                        origins: Some(vec!["https://another.com".to_string()]),
                        methods: Some(vec!["GET".to_string()]),
                        ..Default::default()
                    },
                ],
                ..CORSConfig::default()
            };
            let cors = CORS::from_config(&cors_config).unwrap().unwrap();
            if let CORS::ByOrigin { rules } = &cors {
                assert_eq!(rules.len(), 2);
                assert_eq!(rules[0].origins, vec!["https://example.com"]);
                assert_eq!(rules[1].origins, vec!["https://another.com"]);
            } else {
                panic!("Expected ByOrigin variant");
            }

            // example.com inherits global GET,POST
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://example.com")
                .to_http_request();
            let mut cors_headers = HeaderMap::new();
            cors.set_headers(&req, &mut cors_headers);
            assert_eq!(
                cors_headers
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://example.com"
            );
            assert_eq!(
                cors_headers
                    .get(header::ACCESS_CONTROL_ALLOW_METHODS)
                    .unwrap(),
                "GET, POST"
            );

            // another.com overrides methods to GET only
            let req = TestRequest::with_uri("/graphql")
                .method(Method::POST)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://another.com")
                .to_http_request();
            let mut cors_headers = HeaderMap::new();
            cors.set_headers(&req, &mut cors_headers);

            assert_eq!(
                cors_headers
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "https://another.com"
            );
            assert_eq!(
                cors_headers
                    .get(header::ACCESS_CONTROL_ALLOW_METHODS)
                    .unwrap(),
                "GET"
            );
        }
    }
}
