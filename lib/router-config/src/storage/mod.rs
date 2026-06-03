use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::storage::s3::S3StorageConfig;

pub mod s3;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "type")]
pub enum StorageSourceConfig {
    /// Configuration for an Amazon S3 (or S3-compatible) object storage backend.
    ///
    /// Credentials are optional — if omitted, the client falls back to EC2 instance
    /// metadata (IMDSv2). For explicit credential modes see [`S3Credentials`].
    ///
    /// # Examples
    ///
    /// Minimal configuration relying on EC2 instance role:
    /// ```yaml
    /// bucket: my-bucket
    /// region: eu-west-1
    /// ```
    ///
    /// Localstack / MinIO:
    /// ```yaml
    /// bucket: my-bucket
    /// region: us-east-1
    /// endpoint: http://localhost:4566
    /// allow_http: true
    /// credentials:
    ///   type: static
    ///   access_key_id: test
    ///   secret_access_key: test
    /// ```
    #[serde(rename = "s3")]
    S3(S3StorageConfig),
}

pub type StorageConfigMap = HashMap<String, StorageSourceConfig>;
