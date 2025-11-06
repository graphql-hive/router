use hive_router_config::aws_sig_v4::AwsSigV4SubgraphConfig;
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, ProfileCredentialProvider, RequestSigner,
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
        AwsSigV4SubgraphConfig::DefaultChain(default_chain) => {
            loader = loader.push(DefaultCredentialProvider::new());
            if let Some(profile_name) = &default_chain.profile_name {
                loader = loader.push(ProfileCredentialProvider::new().with_profile(profile_name));
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
