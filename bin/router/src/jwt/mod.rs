pub mod context;
pub mod errors;
pub mod jwks_manager;

use std::{str::FromStr, sync::Arc};

use crate::background_tasks::BackgroundTasksManager;
use crate::config::jwt_auth::{JwtAuthConfig, JwtAuthPluginLookupLocation};
use crate::telemetry::logging::targets;
use cookie::Cookie;
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
                                warn!(target: targets::JWT, error = ?e, "jwt auth failed to convert cookie header to string, ignoring cookie");
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
                                    warn!(target: targets::JWT, error = ?e, "jwt auth failed to parse cookie value, ignoring cookie");
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
        if let Some(jwk) = find_jwks_by_kid(jwt_header, jwks) {
            return Ok(jwk);
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
        header: &Header,
        token: &str,
        jwks: &Vec<Arc<JwkSet>>,
    ) -> Result<JwtTokenPayload, JwtError> {
        let jwk = self.find_matching_jwks(header, jwks)?;

        self.decode_and_validate_token(header, token, &jwk.keys)
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
        // When no audiences are configured, audience validation must be skipped entirely: `jsonwebtoken`
        // defaults `validate_aud` to `true` and rejects any token with an `aud` claim if none is expected.
        match &self.config.audiences {
            Some(aud) => validation.set_audience(aud),
            None => validation.validate_aud = false,
        }

        let token_data = match decode::<JwtClaims>(token, &decoding_key, &validation) {
            Ok(data) => data,
            Err(e) => return Err(JwtError::FailedToDecodeToken(e)),
        };

        match (&self.config.issuers, &token_data.claims.iss) {
            (Some(issuers), Some(token_iss)) if !issuers.contains(token_iss) => {
                return Err(JwtError::FailedToDecodeToken(
                    jsonwebtoken::errors::ErrorKind::InvalidIssuer.into(),
                ));
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
                // First, we need to decode the header to determine which provider to use.
                let header = match decode_header(&token) {
                    Ok(header) => header,
                    Err(err) => return Err(JwtError::InvalidJwtHeader(err)),
                };
                let mut valid_jwks = self.jwks.all();
                if has_unknown_kid(&header, &valid_jwks) {
                    self.jwks.refresh_for_unknown_kid().await;
                    valid_jwks = self.jwks.all();
                }
                self.authenticate(&header, &token, &valid_jwks)
                    .map(Arc::new)
            })
            .await;

        match validation_result {
            Ok(token_payload) => Ok(Some(JwtRequestContext {
                token_payload,
                token_raw: token,
                token_prefix: maybe_prefix,
                scopes_claim: self.config.scopes_claim.clone(),
            })),
            Err(err) => {
                warn!(target: targets::JWT, error = ?err, "jwt token error");
                if self.config.require_authentication.is_some_and(|v| v) {
                    Err((*err).clone())
                } else {
                    Ok(None)
                }
            }
        }
    }
}

/// The key set holding a key with the token's `kid`, if the token has one.
fn find_jwks_by_kid<'a>(header: &Header, jwks: &'a [Arc<JwkSet>]) -> Option<&'a JwkSet> {
    let kid = header.kid.as_deref()?;
    jwks.iter().map(Arc::as_ref).find(|set| {
        set.keys
            .iter()
            .any(|key| key.common.key_id.as_deref() == Some(kid))
    })
}

/// Whether the token names a `kid` that none of the key sets hold, e.g. one signed with a
/// freshly rotated key.
fn has_unknown_kid(header: &Header, jwks: &[Arc<JwkSet>]) -> bool {
    header.kid.is_some() && find_jwks_by_kid(header, jwks).is_none()
}
