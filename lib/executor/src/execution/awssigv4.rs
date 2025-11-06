use hive_router_config::aws_sig_v4::AwsSigV4SubgraphConfig;
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, DefaultCredentialProviderBuilder, RequestSigner,
    StaticCredentialProvider,
};
use reqsign_core::{Context, OsEnv, ProvideCredentialChain, Signer};
use reqsign_file_read_tokio::TokioFileRead;
use reqsign_http_send_reqwest::ReqwestHttpSend;

pub fn create_awssigv4_signer(config: &AwsSigV4SubgraphConfig) -> Signer<Credential> {
    let ctx = Context::new()
        .with_file_read(TokioFileRead)
        .with_http_send(ReqwestHttpSend::default())
        .with_env(OsEnv);
    let mut loader = ProvideCredentialChain::new();
    match config {
        AwsSigV4SubgraphConfig::DefaultChain(default_chain_config) => {
            loader = loader.push(DefaultCredentialProvider::new());
            let mut default_chain_builder = DefaultCredentialProviderBuilder::new();
            if let Some(profile_name) = &default_chain_config.profile_name {
                default_chain_builder = default_chain_builder
                    .configure_profile(|p| p.with_credentials_file(profile_name));
            }
            if let Some(assume_role_config) = &default_chain_config.assume_role {
                default_chain_builder =
                    default_chain_builder.configure_assume_role(|mut assume_role| {
                        assume_role = assume_role.with_role_arn(&assume_role_config.role_arn);
                        if let Some(session_name) = &assume_role_config.session_name {
                            assume_role =
                                assume_role.with_role_session_name(session_name.to_string());
                        }
                        if let Some(region) = &default_chain_config.region {
                            assume_role = assume_role.with_region(region.to_string());
                        }
                        assume_role
                    });
                let default_chain = default_chain_builder.build();
                loader = loader.push(default_chain);
            }
        }
        AwsSigV4SubgraphConfig::HardCoded(hard_coded) => {
            let mut provider = StaticCredentialProvider::new(
                &hard_coded.access_key_id,
                &hard_coded.secret_access_key,
            );
            if let Some(session_token) = &hard_coded.session_token {
                provider = provider.with_session_token(session_token);
            }
            loader = loader.push(provider);
        }
    }
    let service: &str = match config {
        AwsSigV4SubgraphConfig::DefaultChain(default_chain) => {
            default_chain.service.as_ref().map_or("s3", |v| v)
        }
        AwsSigV4SubgraphConfig::HardCoded(hard_coded) => hard_coded.service_name.as_str(),
    };
    let region: &str = match config {
        AwsSigV4SubgraphConfig::DefaultChain(default_chain) => {
            default_chain.region.as_ref().map_or("us-east-1", |v| v)
        }
        AwsSigV4SubgraphConfig::HardCoded(hard_coded) => hard_coded.region.as_str(),
    };
    let builder = RequestSigner::new(service, region);

    Signer::new(ctx, loader, builder)
}
