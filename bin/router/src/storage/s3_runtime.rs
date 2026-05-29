use crate::storage::{error::StorageError, utils::resolve_value_or_expression};
use crate::storage::{StorageGetResult, StorageRuntime};
use async_trait::async_trait;
use hive_router_config::storage::s3::{S3Credentials, S3StorageConfig};
use object_store::aws::{AmazonS3, AmazonS3Builder, AmazonS3ConfigKey};
use object_store::path::Path;
use object_store::{GetOptions, ObjectStore, ObjectStoreExt};
use tracing::warn;

pub struct S3StorageRuntime {
    storage_id: String,
    client: AmazonS3,
}

impl S3StorageRuntime {
    pub fn new(storage_id: &str, config: &S3StorageConfig) -> Result<Self, StorageError> {
        Ok(Self {
            client: Self::build_client(config)?,
            storage_id: storage_id.to_string(),
        })
    }

    fn build_client(config: &S3StorageConfig) -> Result<AmazonS3, StorageError> {
        // Seed the builder from the standard `AWS_*` environment variables so
        // that credentials supplied by the runtime work without any explicit
        // configuration. Most importantly this makes EKS IRSA work out of the
        // box — the pod identity webhook injects `AWS_WEB_IDENTITY_TOKEN_FILE`
        // and `AWS_ROLE_ARN`, which `from_env` picks up — and it also honours
        // plain `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`,
        // `AWS_REGION`, `AWS_ENDPOINT`, and the ECS container-credential vars.
        //
        // The explicit config below is applied afterwards, so any value set in
        // the router config takes precedence over the environment.
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(&resolve_value_or_expression(&config.bucket, "bucket")?);

        if let Some(region) = &config.region {
            builder = builder.with_region(&resolve_value_or_expression(region, "region")?);
        }
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(&resolve_value_or_expression(endpoint, "endpoint")?);
        }
        if let Some(v) = config.virtual_hosted_style {
            builder = builder.with_virtual_hosted_style_request(v);
        }
        if let Some(allow_http) = &config.allow_http {
            builder =
                builder.with_allow_http(resolve_value_or_expression(allow_http, "allow_http")?);
        }

        // Credentials
        match &config.credentials {
            None => {
                // Falls through to IMDSv2 at request time — fine for EC2 instance roles
            }
            Some(S3Credentials::Static {
                access_key_id,
                secret_access_key,
                token,
            }) => {
                builder = builder
                    .with_access_key_id(resolve_value_or_expression(
                        access_key_id,
                        "credentials.access_key_id",
                    )?)
                    .with_secret_access_key(resolve_value_or_expression(
                        secret_access_key,
                        "credentials.secret_access_key",
                    )?);
                if let Some(t) = token {
                    builder =
                        builder.with_token(resolve_value_or_expression(t, "credentials.token")?);
                }
            }
            Some(S3Credentials::WebIdentity {
                token_file,
                role_arn,
                session_name,
                sts_endpoint,
            }) => {
                builder = builder
                    .with_config(
                        AmazonS3ConfigKey::WebIdentityTokenFile,
                        resolve_value_or_expression(token_file, "credentials.token_file")?,
                    )
                    .with_config(
                        AmazonS3ConfigKey::RoleArn,
                        resolve_value_or_expression(role_arn, "credentials.role_arn")?,
                    );
                if let Some(s) = session_name {
                    builder = builder.with_config(
                        AmazonS3ConfigKey::RoleSessionName,
                        resolve_value_or_expression(s, "credentials.session_name")?,
                    );
                }
                if let Some(e) = sts_endpoint {
                    builder = builder.with_config(
                        AmazonS3ConfigKey::StsEndpoint,
                        resolve_value_or_expression(e, "credentials.sts_endpoint")?,
                    );
                }
            }
            Some(S3Credentials::EcsTask { relative_uri }) => {
                builder = builder.with_config(
                    AmazonS3ConfigKey::ContainerCredentialsRelativeUri,
                    resolve_value_or_expression(relative_uri, "credentials.relative_uri")?,
                );
            }
            Some(S3Credentials::EksPodIdentity {
                full_uri,
                token_file,
            }) => {
                builder = builder
                    .with_config(
                        AmazonS3ConfigKey::ContainerCredentialsFullUri,
                        resolve_value_or_expression(full_uri, "credentials.full_uri")?,
                    )
                    .with_config(
                        AmazonS3ConfigKey::ContainerAuthorizationTokenFile,
                        resolve_value_or_expression(token_file, "credentials.token_file")?,
                    );
            }
            Some(S3Credentials::InstanceMetadata {
                imdsv1_fallback,
                metadata_endpoint,
            }) => {
                if let Some(imdsv1_fallback) = imdsv1_fallback {
                    if resolve_value_or_expression(imdsv1_fallback, "credentials.imdsv1_fallback")?
                    {
                        builder = builder.with_imdsv1_fallback();
                    }
                }
                if let Some(ep) = metadata_endpoint {
                    builder = builder.with_metadata_endpoint(resolve_value_or_expression(
                        ep,
                        "credentials.metadata_endpoint",
                    )?);
                }
                // Otherwise nothing to do — this is the default fallback anyway
            }
        }

        // Behavior flags
        if let Some(v) = config.skip_signature {
            builder = builder.with_skip_signature(v);
        }
        if let Some(v) = config.request_payer {
            builder = builder.with_request_payer(v);
        }
        if let Some(v) = config.disable_tagging {
            builder = builder.with_disable_tagging(v);
        }
        if let Some(v) = config.unsigned_payload {
            builder = builder.with_unsigned_payload(v);
        }
        if let Some(v) = config.s3_express {
            builder = builder.with_s3_express(v);
        }

        Ok(builder.build()?)
    }
}

#[async_trait]
impl StorageRuntime for S3StorageRuntime {
    fn identifier(&self) -> &str {
        &self.storage_id
    }

    async fn get(&self, location: &Path) -> Result<(String, Option<String>), StorageError> {
        let response = self.client.get(location).await;

        match response {
            Ok(result) => {
                let etag = result.meta.e_tag.clone();
                let bytes = result.bytes().await?;
                let contents = String::from_utf8(bytes.to_vec())?;

                Ok((contents, etag))
            }
            Err(e) => {
                warn!(error = %e, "failed to load contents from s3");

                Err(e.into())
            }
        }
    }

    async fn get_if_none_changed(
        &self,
        location: &Path,
        if_none_match: Option<String>,
    ) -> Result<StorageGetResult, StorageError> {
        let response = self
            .client
            .get_opts(
                location,
                GetOptions {
                    if_none_match,
                    ..Default::default()
                },
            )
            .await;

        match response {
            Ok(result) => {
                let etag = result.meta.e_tag.clone();
                let bytes = result.bytes().await?;
                let contents = String::from_utf8(bytes.to_vec())?;

                Ok(StorageGetResult::Modified { contents, etag })
            }
            Err(object_store::Error::NotModified { .. }) => Ok(StorageGetResult::NotModified),
            Err(e) => {
                warn!(error = %e, "failed to load contents from s3");

                Err(e.into())
            }
        }
    }
}
