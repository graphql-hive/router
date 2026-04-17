use std::sync::Arc;

use hive_router_config::traffic_shaping::ServerTLSConfig;
use hive_router_plan_executor::executors::{
    error::TlsCertificatesError, map::from_cert_file_config_to_certificate_der,
};
use rustls::{
    pki_types::{pem::PemObject, PrivateKeyDer},
    server::{NoClientAuth, WebPkiClientVerifier},
    RootCertStore, ServerConfig,
};

pub fn build_rustls_config(
    tls_config: &ServerTLSConfig,
) -> Result<ServerConfig, TlsCertificatesError> {
    let client_auth = if let Some(client_auth_config) = tls_config.client_auth.as_ref() {
        let certs = from_cert_file_config_to_certificate_der(&client_auth_config.cert_file)?;
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(certs);
        let builder = WebPkiClientVerifier::builder(roots.into());
        let required = client_auth_config.required.unwrap_or(true);
        if required {
            builder.build()?
        } else {
            builder.allow_unauthenticated().build()?
        }
    } else {
        Arc::new(NoClientAuth)
    };
    let certs = from_cert_file_config_to_certificate_der(&tls_config.cert_file)?;
    let key = PrivateKeyDer::from_pem_file(&tls_config.key_file.absolute)
        .map_err(|err| TlsCertificatesError::CustomTlsCertificatesError("key_file", err))?;
    Ok(ServerConfig::builder()
        .with_client_cert_verifier(client_auth)
        .with_single_cert(certs, key)?)
}
