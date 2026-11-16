use dashmap::DashMap;
use futures::future::{BoxFuture, FutureExt, Shared};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::config::{
    primitives::{file_path::FilePath, single_or_multiple::SingleOrMultiple},
    traffic_shaping::ClientTLSConfig,
};
use hyper_rustls::{ConfigBuilderExt, HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::dns::{GaiResolver, Name};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tower_service::Service;

use crate::executor::executors::error::{SubgraphExecutorError, TlsCertificatesError};

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

pub type HttpConnector = hyper_util::client::legacy::connect::HttpConnector<SharedResolver>;

type InFlightLookup = Shared<BoxFuture<'static, Result<Vec<SocketAddr>, Arc<io::Error>>>>;

/// hyper's default resolver runs getaddrinfo on a blocking thread for every new connection, so a
/// burst of new subgraph connections starts hundreds of threads, each with its own stack. This
/// one lets connections that ask for the same name while a lookup is running wait for that lookup
/// and take its answer. Nothing is kept after that, the next connection looks the name up again,
/// so the OS resolver still decides how long an answer is good for.
#[derive(Clone, Default)]
pub struct SharedResolver {
    in_flight: Arc<DashMap<Name, InFlightLookup>>,
}

impl SharedResolver {
    async fn resolve<F, Fut>(&self, name: Name, lookup: F) -> io::Result<Vec<SocketAddr>>
    where
        F: FnOnce(Name) -> Fut,
        Fut: Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'static,
    {
        let mut started_here = false;
        let lookup = self
            .in_flight
            .entry(name.clone())
            .or_insert_with(|| {
                started_here = true;
                // only a Weak to the map, so a dropped resolver doesn't stay alive through a
                // lookup that's still running
                let in_flight = Arc::downgrade(&self.in_flight);
                let started = lookup(name.clone());
                async move {
                    // a panicking lookup is an error for everyone waiting, not a panic in each
                    let addrs = AssertUnwindSafe(started)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| Err(io::Error::other("name lookup panicked")))
                        // a failed lookup isn't kept either, the next connection tries again
                        .map_err(Arc::new);
                    if let Some(in_flight) = in_flight.upgrade() {
                        in_flight.remove(&name);
                    }
                    addrs
                }
                .boxed()
                .shared()
            })
            .clone();
        // the map guard is gone by now. The lookup also runs on its own task, so it finishes and
        // clears its entry even when every connection waiting on it gives up.
        if started_here {
            tokio::spawn(lookup.clone());
        }
        lookup.await.map_err(|err| io::Error::new(err.kind(), err))
    }
}

impl Service<Name> for SharedResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<Self::Response>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let addrs = this
                .resolve(name, |name| async move {
                    Ok(GaiResolver::new().call(name).await?.collect())
                })
                .await?;
            Ok(addrs.into_iter())
        })
    }
}

pub fn build_https_connector(
    tls_config: Option<&ClientTLSConfig>,
) -> Result<HttpsConnector<HttpConnector>, SubgraphExecutorError> {
    let mut http = HttpConnector::new_with_resolver(SharedResolver::default());
    http.enforce_http(false);
    http.set_nodelay(true);
    Ok(HttpsConnectorBuilder::new()
        .with_tls_config(build_https_client_config(tls_config)?)
        .https_or_http()
        .enable_all_versions()
        .wrap_connector(http))
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn instant_lookups_under_contention_finish_and_leave_nothing() {
        let resolver = SharedResolver::default();
        let addr = SocketAddr::from(([10, 0, 0, 1], 0));
        let tasks: Vec<_> = (0..2000)
            .map(|i| {
                let resolver = resolver.clone();
                let name = Name::from_str(["products", "reviews", "users"][i % 3]).unwrap();
                tokio::spawn(async move {
                    resolver
                        .resolve(name, |_| async move { Ok(vec![addr]) })
                        .await
                        .unwrap()
                })
            })
            .collect();
        tokio::time::timeout(Duration::from_secs(5), async {
            for task in tasks {
                assert_eq!(task.await.unwrap(), vec![addr]);
            }
        })
        .await
        .expect("lookups got stuck");
        // a lookup takes its entry out before anyone gets its answer
        assert!(resolver.in_flight.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connections_waiting_on_a_failed_lookup_share_its_error() {
        let resolver = SharedResolver::default();
        let lookups = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..50)
            .map(|_| {
                let resolver = resolver.clone();
                let lookups = lookups.clone();
                tokio::spawn(async move {
                    resolver
                        .resolve(Name::from_str("products").unwrap(), |_| async move {
                            lookups.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Err(io::Error::new(io::ErrorKind::NotFound, "no such host"))
                        })
                        .await
                })
            })
            .collect();
        for task in tasks {
            let err = task.await.unwrap().unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        }
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
        assert!(resolver.in_flight.is_empty());
    }

    #[tokio::test]
    async fn lookup_nobody_waits_for_anymore_still_clears_its_entry() {
        let resolver = SharedResolver::default();
        let in_flight = Arc::downgrade(&resolver.in_flight);
        let gave_up = tokio::time::timeout(
            Duration::from_millis(5),
            resolver.resolve(Name::from_str("products").unwrap(), |_| async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok(vec![SocketAddr::from(([10, 0, 0, 1], 0))])
            }),
        )
        .await;
        assert!(gave_up.is_err());
        assert_eq!(resolver.in_flight.len(), 1);

        // the lookup finishes on its own and takes its entry out
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(resolver.in_flight.is_empty());

        // and a running lookup doesn't keep a dropped resolver's map alive
        let _ = tokio::time::timeout(
            Duration::from_millis(5),
            resolver.resolve(Name::from_str("reviews").unwrap(), |_| async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok(vec![])
            }),
        )
        .await;
        drop(resolver);
        assert!(in_flight.upgrade().is_none());
    }

    #[tokio::test]
    async fn connections_to_one_name_share_one_lookup() {
        let resolver = SharedResolver::default();
        let lookups = Arc::new(AtomicUsize::new(0));
        let addr = SocketAddr::from(([10, 0, 0, 1], 0));

        let tasks: Vec<_> = (0..50)
            .map(|_| {
                let resolver = resolver.clone();
                let lookups = lookups.clone();
                tokio::spawn(async move {
                    resolver
                        .resolve(Name::from_str("products").unwrap(), |_| async move {
                            lookups.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Ok(vec![addr])
                        })
                        .await
                        .unwrap()
                })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), vec![addr]);
        }
        assert_eq!(lookups.load(Ordering::SeqCst), 1);

        // once that lookup is done nothing is kept, the next connection looks it up again
        resolver
            .resolve(Name::from_str("products").unwrap(), |_| {
                lookups.fetch_add(1, Ordering::SeqCst);
                async move { Ok(vec![addr]) }
            })
            .await
            .unwrap();
        assert_eq!(lookups.load(Ordering::SeqCst), 2);

        // a failed lookup isn't kept
        let failed = resolver
            .resolve(Name::from_str("reviews").unwrap(), |_| async {
                Err(io::Error::other("nope"))
            })
            .await;
        assert!(failed.is_err());
        let retried = resolver
            .resolve(Name::from_str("reviews").unwrap(), |_| async move {
                Ok(vec![addr])
            })
            .await
            .unwrap();
        assert_eq!(retried, vec![addr]);
    }
}
