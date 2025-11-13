use hive_router_config::aws_sig_v4::AwsSigV4SubgraphConfig;
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, DefaultCredentialProviderBuilder, RequestSigner,
    StaticCredentialProvider,
};
use reqsign_core::{Context, OsEnv, ProvideCredentialChain, Signer};
use reqsign_file_read_tokio::TokioFileRead;
use reqsign_http_send_reqwest::ReqwestHttpSend;

pub fn create_awssigv4_signer(config: &AwsSigV4SubgraphConfig) -> Option<Signer<Credential>> {
    let ctx = Context::new()
        .with_file_read(TokioFileRead)
        .with_http_send(ReqwestHttpSend::default())
        .with_env(OsEnv);
    let mut loader = ProvideCredentialChain::new();
    match config {
        AwsSigV4SubgraphConfig::Disabled => {
            return None;
        }
        AwsSigV4SubgraphConfig::DefaultChain {
            default_chain: default_chain_config,
        } => {
            loader = loader.push(DefaultCredentialProvider::new());
            let mut default_chain_builder = DefaultCredentialProviderBuilder::new();
            if let Some(profile_name) = &default_chain_config.profile_name {
                default_chain_builder = default_chain_builder
                    .configure_profile(|p| p.with_credentials_file(profile_name));
            }
            if let Some(assume_role_config) = &default_chain_config.assume_role {
                default_chain_builder =
                    default_chain_builder.configure_assume_role(|mut assume_role| {
                        assume_role = assume_role
                            .with_role_arn(&assume_role_config.role_arn)
                            .with_region(default_chain_config.region.to_string());
                        if let Some(session_name) = &assume_role_config.session_name {
                            assume_role =
                                assume_role.with_role_session_name(session_name.to_string());
                        }
                        assume_role
                    });
                let default_chain = default_chain_builder.build();
                loader = loader.push(default_chain);
            }
        }
        AwsSigV4SubgraphConfig::HardCoded { hardcoded } => {
            let mut provider = StaticCredentialProvider::new(
                &hardcoded.access_key_id,
                &hardcoded.secret_access_key,
            );
            if let Some(session_token) = &hardcoded.session_token {
                provider = provider.with_session_token(session_token);
            }
            loader = loader.push(provider);
        }
    }
    let service: &str = match config {
        AwsSigV4SubgraphConfig::DefaultChain { default_chain } => &default_chain.service,
        AwsSigV4SubgraphConfig::HardCoded { hardcoded } => &hardcoded.service_name,
        AwsSigV4SubgraphConfig::Disabled => unreachable!(),
    };
    let region: &str = match config {
        AwsSigV4SubgraphConfig::DefaultChain { default_chain } => &default_chain.region,
        AwsSigV4SubgraphConfig::HardCoded { hardcoded } => &hardcoded.region,
        AwsSigV4SubgraphConfig::Disabled => unreachable!(),
    };
    let builder = RequestSigner::new(service, region);

    Some(Signer::new(ctx, loader, builder))
}

#[cfg(test)]
mod tests {
    use crate::execution::awssigv4::create_awssigv4_signer;
    use bytes::Bytes;
    use chrono::Utc;
    use hive_router_config::aws_sig_v4::{AwsSigV4SubgraphConfig, HardCodedConfig};
    use http_body_util::Full;
    use hyper::body::Body;

    #[tokio::test]
    async fn signs_the_request_correctly() {
        let access_key_id = "AKIAIOSFODNN7EXAMPLE";
        let secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let region = "eu-central-1";
        let service_name = "s3";
        let config = AwsSigV4SubgraphConfig::HardCoded {
            hardcoded: HardCodedConfig {
                access_key_id: access_key_id.to_string(),
                secret_access_key: secret_access_key.to_string(),
                region: region.to_string(),
                service_name: service_name.to_string(),
                session_token: None,
            },
        };
        let signer = create_awssigv4_signer(&config).expect("Expected to return a signer");
        let body = Full::new(Bytes::from("query { hello }"));
        let content_length = body.size_hint().exact().unwrap();
        let endpoint = format!(
            "http://sigv4examplegraphqlbucket.{}-{}.amazonaws.com",
            service_name, region
        );
        let req: http::Request<Full<Bytes>> = http::Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("Accept", "application/json")
            .header("Content-Length", content_length)
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();

        let (mut parts, body) = req.into_parts();

        signer
            .sign(&mut parts, None)
            .await
            .expect("Expected to sign correctly");

        let req = http::Request::from_parts(parts, body);

        let authorization_header = req
            .headers()
            .get("Authorization")
            .expect("Expected to have Authorization header")
            .to_str()
            .expect("Expected to convert to str");

        let date_stamp = Utc::now().format("%Y%m%d");

        let mut expected_auth_header_prefix = "AWS4-HMAC-SHA256 ".to_string();
        expected_auth_header_prefix.push_str(&format!(
            "Credential={}/{}/{}/{}/aws4_request, ",
            access_key_id, date_stamp, region, service_name
        ));
        expected_auth_header_prefix.push_str(
            "SignedHeaders=accept;content-length;content-type;host;x-amz-content-sha256;x-amz-date, Signature=",
        );

        assert!(
            authorization_header.starts_with(&expected_auth_header_prefix),
            "Expected authorization header to start with '{}', but got '{}'",
            expected_auth_header_prefix,
            authorization_header
        );
    }
}
