use std::sync::Arc;

use hive_router_config::{
    primitives::{file_path::FilePath, single_or_multiple::SingleOrMultiple},
    traffic_shaping::ClientTLSConfig,
};
use hyper_rustls::{ConfigBuilderExt, HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};

use crate::executors::error::{SubgraphExecutorError, TlsCertificatesError};

pub fn from_cert_file_config_to_certificate_der<'a>(
    cert_file_path: &SingleOrMultiple<FilePath>,
) -> Result<Vec<CertificateDer<'a>>, TlsCertificatesError> {
    match cert_file_path {
        SingleOrMultiple::Single(cert_file_path) => {
            CertificateDer::pem_file_iter(&cert_file_path.absolute)
                .and_then(|res| res.collect::<Result<Vec<_>, _>>())
                .map_err(|err| TlsCertificatesError::CustomTlsCertificatesError("cert_file", err))
        }
        SingleOrMultiple::Multiple(file_paths) => file_paths
            .iter()
            .map(|file_path| {
                CertificateDer::pem_file_iter(&file_path.absolute)
                    .and_then(|res| res.collect::<Result<Vec<_>, _>>())
                    .map_err(|err| {
                        TlsCertificatesError::CustomTlsCertificatesError("cert_file", err)
                    })
            })
            .try_fold(Vec::new(), |mut acc, certs_result| {
                certs_result.map(|mut certs| {
                    acc.append(&mut certs);
                    acc
                })
            }),
    }
}

/// A certificate verifier that accepts any server certificate without validation.
/// Only for use in development/testing environments.
#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

pub fn build_https_client_config(
    tls_config: Option<&ClientTLSConfig>,
) -> Result<ClientConfig, TlsCertificatesError> {
    let insecure_skip = tls_config
        .map(|c| c.insecure_skip_ca_verification)
        .unwrap_or(false);

    let tls_config_for_rustls = if insecure_skip {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
    } else if let Some(cert_file_path) = tls_config.and_then(|c| c.cert_file.as_ref()) {
        // Read trust roots
        let certs = from_cert_file_config_to_certificate_der(cert_file_path)?;

        if certs.is_empty() {
            return Err(TlsCertificatesError::InvalidTlsCertificates(format!(
                "No valid certificates found in {:#?}",
                cert_file_path
            )));
        }

        let certs_len = certs.len();
        let mut roots = RootCertStore::empty();
        let (valid, _) = roots.add_parsable_certificates(certs);
        if valid != certs_len {
            return Err(TlsCertificatesError::InvalidTlsCertificates(format!(
                "Expected {} certificates in {:#?}, but only {} were valid",
                certs_len, cert_file_path, valid
            )));
        }
        // TLS client config using the custom CA store for lookups
        ClientConfig::builder().with_root_certificates(roots)
    } else {
        ClientConfig::builder()
            .with_native_roots()
            .map_err(TlsCertificatesError::NativeTlsCertificatesError)?
    };
    let client_config = if let Some(client_auth) = tls_config.and_then(|c| c.client_auth.as_ref()) {
        let certs = from_cert_file_config_to_certificate_der(&client_auth.cert_file)?;

        let private_key =
            PrivateKeyDer::from_pem_file(&client_auth.key_file.absolute).map_err(|err| {
                TlsCertificatesError::CustomTlsCertificatesError("client_auth.key", err)
            })?;

        tls_config_for_rustls
            .with_client_auth_cert(certs, private_key)
            .map_err(TlsCertificatesError::TlsConfigFailure)?
    } else {
        tls_config_for_rustls.with_no_client_auth()
    };

    Ok(client_config)
}

pub fn build_https_connector(
    tls_config: Option<&ClientTLSConfig>,
) -> Result<HttpsConnector<HttpConnector>, SubgraphExecutorError> {
    Ok(HttpsConnectorBuilder::new()
        .with_tls_config(build_https_client_config(tls_config)?)
        .https_or_http()
        .enable_all_versions()
        .build())
}

pub fn get_merged_tls_config(
    global: Option<&ClientTLSConfig>,
    subgraph: Option<&ClientTLSConfig>,
) -> Option<ClientTLSConfig> {
    match (global, subgraph) {
        (Some(global), Some(subgraph)) => {
            // If both global and subgraph TLS configs are provided, we merge them by giving precedence to subgraph config values.
            // If the subgraph config has a field set to None, we fall back to the global config for that field.
            let merged = ClientTLSConfig {
                cert_file: subgraph
                    .cert_file
                    .clone()
                    .or_else(|| global.cert_file.clone()),
                client_auth: subgraph
                    .client_auth
                    .clone()
                    .or_else(|| global.client_auth.clone()),
                insecure_skip_ca_verification: subgraph.insecure_skip_ca_verification
                    || global.insecure_skip_ca_verification,
            };
            Some(merged)
        }
        (None, Some(subgraph)) => Some(subgraph.clone()),
        (Some(global), None) => Some(global.clone()),
        (None, None) => None,
    }
}
