pub mod context;
pub mod errors;
pub mod jwks_manager;

use std::{str::FromStr, sync::Arc};

use cookie::Cookie;
use hive_router_config::jwt_auth::{JwtAuthConfig, JwtAuthPluginLookupLocation};
use hive_router_internal::background_tasks::BackgroundTasksManager;
use http::header::COOKIE;
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet},
    Algorithm, DecodingKey, Header, Validation,
};
use ntex::{http::header::HeaderValue, http::HeaderMap};
use tracing::warn;

use crate::{
    jwt::{
        context::{Audience, JwtClaims, JwtRequestContext, JwtTokenPayload},
        errors::{JwtError, LookupError},
        jwks_manager::{JwksManager, JwksSourceError},
    },
    shared_state::JwtClaimsCache,
};

pub struct JwtAuthRuntime {
    config: JwtAuthConfig,
    jwks: JwksManager,
}

impl JwtAuthRuntime {
    pub async fn init(
        background_tasks_mgr: &mut BackgroundTasksManager,
        config: &JwtAuthConfig,
    ) -> Result<Self, JwksSourceError> {
        let jwks = JwksManager::from_config(config);

        // If any of the sources needs to be prefetched (loaded when the server starts), then we'll
        // try to load it now, and fail if it fails.
        jwks.prefetch_sources().await?;

        // Register background tasks for refreshing JWKS keys
        jwks.register_background_tasks(background_tasks_mgr);

        let instance = JwtAuthRuntime {
            config: config.clone(),
            jwks,
        };

        Ok(instance)
    }

    fn lookup(&self, headers: &HeaderMap) -> Result<(Option<String>, String), LookupError> {
        for lookup_config in &self.config.lookup_locations {
            match lookup_config {
                JwtAuthPluginLookupLocation::Header { name, prefix } => {
                    if let Some(header_value) = headers.get(name.get_header_ref()) {
                        let header_str = match header_value.to_str() {
                            Ok(s) => s,
                            Err(e) => return Err(LookupError::FailedToStringifyHeader(e)),
                        };

                        let header_value: HeaderValue = match header_str.parse() {
                            Ok(v) => v,
                            Err(e) => return Err(LookupError::FailedToParseHeader(e)),
                        };

                        match prefix {
                            Some(prefix) => match header_value
                                .to_str()
                                .ok()
                                .and_then(|s| s.strip_prefix(prefix))
                            {
                                Some(stripped_value) => {
                                    return Ok((
                                        Some(prefix.to_string()),
                                        stripped_value.trim().to_string(),
                                    ));
                                }
                                None => {
                                    return Err(LookupError::MismatchedPrefix);
                                }
                            },
                            None => {
                                return Ok((None, header_value.to_str().unwrap_or("").to_string()));
                            }
                        }
                    }
                }
                JwtAuthPluginLookupLocation::Cookie { name } => {
                    if let Some(cookie_raw) = headers.get(COOKIE) {
                        let raw_cookies = match cookie_raw.to_str() {
                            Ok(cookies) => cookies.split(';'),
                            Err(e) => {
                                warn!("jwt auth failed to convert cookie header to string, ignoring cookie. error: {}", e);
                                continue;
                            }
                        };

                        for item in raw_cookies {
                            match Cookie::parse(item) {
                                Ok(v) => {
                                    let (cookie_name, cookie_value) = v.name_value_trimmed();

                                    if cookie_name == name {
                                        return Ok((None, cookie_value.to_string()));
                                    }
                                }
                                Err(e) => {
                                    // Should we reject the entire request in case of invalid cookies?
                                    // I think it's better to consider this as a user error? maybe return 400?
                                    warn!(
                       "jwt auth failed to parse cookie value, ignoring cookie. error: {}",
                       e
                     );
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(LookupError::LookupFailed)
    }

    pub(crate) fn find_matching_jwks<'a>(
        &'a self,
        jwt_header: &Header,
        jwks: &'a Vec<Arc<JwkSet>>,
    ) -> Result<&'a JwkSet, JwtError> {
        // If `kid` is vailable on the header, we can try to match it to the `kid` on the available JWKs.
        if let Some(jwt_kid) = &jwt_header.kid {
            for jwk in jwks {
                for key in &jwk.keys {
                    if key.common.key_id.as_ref().is_some_and(|v| v == jwt_kid) {
                        return Ok(jwk);
                    }
                }
            }
        }

        // If we don't have `kid` on the token, we should try to match the `alg` field.
        for jwk in jwks {
            for key in &jwk.keys {
                if let Some(key_alg) = key.common.key_algorithm {
                    let key_alg_cmp = Algorithm::from_str(&key_alg.to_string())
                        .map_err(JwtError::JwkAlgorithmNotSupported)?;
                    if key_alg_cmp == jwt_header.alg {
                        return Ok(jwk);
                    }
                }
            }
        }

        Err(JwtError::FailedToLocateProvider)
    }

    fn authenticate(
        &self,
        jwks: &Vec<Arc<JwkSet>>,
        headers: &HeaderMap,
    ) -> Result<(JwtTokenPayload, Option<String>, String), JwtError> {
        match self.lookup(headers) {
            Ok((maybe_prefix, token)) => {
                // First, we need to decode the header to determine which provider to use.
                let header = decode_header(&token).map_err(JwtError::InvalidJwtHeader)?;
                let jwk = self.find_matching_jwks(&header, jwks)?;

                self.decode_and_validate_token(&header, &token, &jwk.keys)
                    .map(|token_data| (token_data, maybe_prefix, token))
            }
            Err(e) => {
                warn!("jwt plugin failed to lookup token. error: {}", e);

                Err(JwtError::LookupFailed(e))
            }
        }
    }

    fn decode_and_validate_token(
        &self,
        header: &Header,
        token: &str,
        jwks: &[Jwk],
    ) -> Result<JwtTokenPayload, JwtError> {
        let decode_attempts = jwks
            .iter()
            .map(|jwk| self.try_decode_from_jwk(header, token, jwk));

        if let Some(success) = decode_attempts.clone().find(|result| result.is_ok()) {
            return success;
        }

        Err(JwtError::AllProvidersFailedToDecode(
            decode_attempts
                .into_iter()
                .map(|result: Result<JwtTokenPayload, JwtError>| result.unwrap_err())
                .collect::<Vec<_>>(),
        ))
    }

    fn try_decode_from_jwk(
        &self,
        header: &Header,
        token: &str,
        jwk: &Jwk,
    ) -> Result<JwtTokenPayload, JwtError> {
        let decoding_key = DecodingKey::from_jwk(jwk).map_err(JwtError::InvalidDecodingKey)?;

        let alg = match jwk.common.key_algorithm {
            Some(key_alg) => Algorithm::from_str(&key_alg.to_string())
                .map_err(JwtError::JwkAlgorithmNotSupported)?,
            None => header.alg,
        };

        // Make sure the algorithm is in the allowed algorithms before proceeding
        if let Some(allowed) = &self.config.allowed_algorithms {
            if !allowed.contains(&alg) {
                return Err(JwtError::JwkAlgorithmNotSupported(
                    jsonwebtoken::errors::ErrorKind::InvalidAlgorithm.into(),
                ));
            }
        }

        let mut validation = Validation::new(alg);

        // This only validates the existence of the claim, it does not validate the values, we'll do it after decoding.
        if let Some(iss) = &self.config.issuers {
            validation.set_issuer(iss);
        }

        // This only validates the existence of the claim, it does not validate the values, we'll do it after decoding.
        if let Some(aud) = &self.config.audiences {
            validation.set_audience(aud);
        }

        let token_data = match decode::<JwtClaims>(token, &decoding_key, &validation) {
            Ok(data) => data,
            Err(e) => return Err(JwtError::FailedToDecodeToken(e)),
        };

        match (&self.config.issuers, &token_data.claims.iss) {
            (Some(issuers), Some(token_iss)) => {
                if !issuers.contains(token_iss) {
                    return Err(JwtError::FailedToDecodeToken(
                        jsonwebtoken::errors::ErrorKind::InvalidIssuer.into(),
                    ));
                }
            }
            (Some(_), None) => {
                return Err(JwtError::FailedToDecodeToken(
                    jsonwebtoken::errors::ErrorKind::InvalidIssuer.into(),
                ));
            }
            _ => {}
        };

        match (&self.config.audiences, &token_data.claims.aud) {
            (Some(audiences), Some(token_aud)) => {
                let all_valid = match token_aud {
                    Audience::Single(s) => audiences.contains(s),
                    Audience::Multiple(s) => s.iter().all(|v| audiences.contains(v)),
                };

                if !all_valid {
                    return Err(JwtError::FailedToDecodeToken(
                        jsonwebtoken::errors::ErrorKind::InvalidAudience.into(),
                    ));
                }
            }
            (Some(_), None) => {
                return Err(JwtError::FailedToDecodeToken(
                    jsonwebtoken::errors::ErrorKind::InvalidAudience.into(),
                ));
            }
            _ => {}
        };

        Ok(token_data)
    }

    pub async fn validate_headers(
        &self,
        headers: &HeaderMap,
        cache: &JwtClaimsCache,
    ) -> Result<Option<JwtRequestContext>, JwtError> {
        let (maybe_prefix, token) = match self.lookup(headers) {
            Ok((p, t)) => (p, t),
            Err(e) => {
                // No token found, but this is only an error if auth is required.
                if self.config.require_authentication.is_some_and(|v| v) {
                    return Err(JwtError::LookupFailed(e));
                }
                return Ok(None);
            }
        };

        let validation_result = cache
            .try_get_with(token.clone(), async {
                let valid_jwks = self.jwks.all();
                self.authenticate(&valid_jwks, headers)
                    .map(|(payload, _, _)| Arc::new(payload))
            })
            .await;

        match validation_result {
            Ok(token_payload) => Ok(Some(JwtRequestContext {
                token_payload,
                token_raw: token,
                token_prefix: maybe_prefix,
            })),
            Err(err) => {
                warn!("jwt token error: {:?}", err);
                if self.config.require_authentication.is_some_and(|v| v) {
                    Err((*err).clone())
                } else {
                    Ok(None)
                }
            }
        }
    }
}
