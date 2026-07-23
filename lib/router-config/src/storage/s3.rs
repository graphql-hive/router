use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct S3StorageConfig {
    /// Name of the S3 bucket to read from.
    pub bucket: ValueOrExpression<String>,

    /// AWS region the bucket resides in, e.g. `us-east-1` or `eu-west-1`.
    ///
    /// When using a custom
    /// [`endpoint`](Self::endpoint) pointing at a non-AWS service (Localstack,
    /// MinIO, Cloudflare R2), set this to whatever region that service expects —
    /// typically `us-east-1` or `auto` (R2).
    pub region: Option<ValueOrExpression<String>>,

    /// Custom endpoint URL for the S3 API, e.g. `http://localhost:4566` for
    /// [Localstack](https://localstack.cloud) or `http://minio:9000` for
    /// [MinIO](https://min.io).
    ///
    /// When set, the bucket name is appended as a path segment by default
    /// (`<endpoint>/<bucket>`). If the service expects virtual-hosted-style
    /// requests (`<bucket>.<host>`), enable [`virtual_hosted_style`](Self::virtual_hosted_style).
    ///
    /// HTTP endpoints also require [`allow_http`](Self::allow_http) to be `true`.
    pub endpoint: Option<ValueOrExpression<String>>,

    /// Use [virtual-hosted-style requests](https://docs.aws.amazon.com/AmazonS3/latest/userguide/VirtualHosting.html)
    /// (`<bucket>.<host>/key`) instead of the default path-style
    /// (`<host>/<bucket>/key`).
    ///
    /// Must be consistent with [`endpoint`](Self::endpoint): if virtual-hosted
    /// style is enabled, the endpoint should already include the bucket name in
    /// the host.
    ///
    /// Defaults to `false`.
    pub virtual_hosted_style: Option<bool>,

    /// Allow plain HTTP connections in addition to HTTPS.
    ///
    /// **Warning:** enabling this exposes requests and credentials to
    /// network interception. Only use for local development or fully trusted
    /// private networks.
    ///
    /// Required when [`endpoint`](Self::endpoint) uses an `http://` URL, otherwise requests will fail.
    ///
    /// Defaults to `false`.
    pub allow_http: Option<ValueOrExpression<bool>>,

    /// Credential provider to authenticate with S3.
    ///
    /// When omitted, credentials are resolved from the standard `AWS_*`
    /// environment variables and the ambient runtime, matching the behaviour of
    /// the AWS SDKs. In particular:
    ///
    /// - On **EKS**, [IAM Roles for Service Accounts (IRSA)](https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html)
    ///   works out of the box: the pod identity webhook injects
    ///   `AWS_WEB_IDENTITY_TOKEN_FILE` and `AWS_ROLE_ARN`, which are picked up
    ///   automatically — no `credentials` block required.
    /// - `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (and optional
    ///   `AWS_SESSION_TOKEN`) provide static credentials.
    /// - ECS task roles and EKS Pod Identity are resolved from their respective
    ///   `AWS_CONTAINER_CREDENTIALS_*` variables.
    /// - On EC2, the client finally falls through to
    ///   [Instance Metadata Service (IMDSv2)](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html).
    ///
    /// When this field **is** set, credentials are taken solely from the config:
    /// the `AWS_*` credential environment variables are ignored entirely, so they
    /// cannot mix into or shadow the mode you configured (only non-credential
    /// settings such as region and endpoint still fall back to the environment).
    ///
    /// See [`S3Credentials`] for all supported authentication modes.
    pub credentials: Option<S3Credentials>,

    /// Skip request signing entirely.
    ///
    /// Useful for public buckets that reject signed requests. When `true`, no
    /// credentials are fetched or sent.
    ///
    /// See [`AmazonS3Builder::with_skip_signature`](https://docs.rs/object_store/latest/object_store/aws/struct.AmazonS3Builder.html#method.with_skip_signature).
    /// Defaults to `false`.
    pub skip_signature: Option<bool>,

    /// Charge the requester (rather than the bucket owner) for request and
    /// data transfer costs.
    ///
    /// Required for access to
    /// [Requester Pays buckets](https://docs.aws.amazon.com/AmazonS3/latest/userguide/RequesterPaysBuckets.html).
    /// Defaults to `false`.
    pub request_payer: Option<bool>,

    /// Disable object tagging on writes.
    ///
    /// Some S3-compatible services do not support the tagging API. Setting this
    /// to `true` suppresses tagging headers on all `PUT` requests.
    ///
    /// Defaults to `false`.
    pub disable_tagging: Option<bool>,

    /// Use the [`UNSIGNED-PAYLOAD`](https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html)
    /// literal when computing request signatures, skipping body checksumming.
    ///
    /// Can reduce CPU overhead for large uploads at the cost of payload
    /// integrity verification. Defaults to `false`.
    pub unsigned_payload: Option<bool>,

    /// Enable support for
    /// [S3 Express One Zone](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-express-one-zone.html)
    /// directory buckets.
    ///
    /// When `true`, the bucket name must follow the S3 Express naming
    /// convention (e.g. `my-bucket--use1-az4--x-s3`). Defaults to `false`.
    pub s3_express: Option<bool>,
}

/// Authentication mode for S3.
///
/// The builder resolves credentials in the following priority order:
///
/// 1. [`Static`](Self::Static) — explicit access key + secret
/// 2. [`WebIdentity`](Self::WebIdentity) — EKS IRSA via STS `AssumeRoleWithWebIdentity`
/// 3. [`EcsTask`](Self::EcsTask) — ECS task IAM role
/// 4. [`EksPodIdentity`](Self::EksPodIdentity) — EKS Pod Identity
/// 5. [`InstanceMetadata`](Self::InstanceMetadata) — EC2 IMDSv2
///
/// When `credentials` is omitted entirely, credentials are instead resolved
/// from the `AWS_*` environment variables and the ambient runtime — see
/// [`S3StorageConfig::credentials`].
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum S3Credentials {
    /// Long-lived or temporary static credentials.
    ///
    /// Suitable for local development, CI, or workloads outside AWS. For
    /// temporary credentials (e.g. from `aws sts assume-role`), supply the
    /// `token` field as well.
    ///
    /// ```yaml
    /// credentials:
    ///   type: static
    ///   access_key_id: AKIAIOSFODNN7EXAMPLE
    ///   secret_access_key: wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
    /// ```
    Static {
        /// AWS access key ID, e.g. `AKIAIOSFODNN7EXAMPLE`.
        access_key_id: ValueOrExpression<String>,

        /// AWS secret access key corresponding to `access_key_id`.
        secret_access_key: ValueOrExpression<String>,

        /// Session token for temporary credentials obtained via
        /// [`AssumeRole`](https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRole.html)
        /// or `aws sts assume-role`. Omit for long-lived IAM user credentials.
        token: Option<ValueOrExpression<String>>,
    },

    /// [IAM Roles for Service Accounts (IRSA)](https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html)
    /// on EKS via `AssumeRoleWithWebIdentity`.
    ///
    /// The Kubernetes service account token is exchanged for temporary AWS
    /// credentials through STS. Typically the token file and role ARN are
    /// injected by the EKS pod identity webhook via environment variables
    /// (`AWS_WEB_IDENTITY_TOKEN_FILE`, `AWS_ROLE_ARN`).
    ///
    /// ```yaml
    /// credentials:
    ///   type: web_identity
    ///   token_file: /var/run/secrets/eks.amazonaws.com/serviceaccount/token
    ///   role_arn: arn:aws:iam::123456789012:role/MyServiceRole
    /// ```
    WebIdentity {
        /// Path to the file containing the web identity token (a JWT issued by
        /// the Kubernetes OIDC provider).
        ///
        /// Typically `/var/run/secrets/eks.amazonaws.com/serviceaccount/token`.
        token_file: ValueOrExpression<String>,

        /// ARN of the IAM role to assume, e.g.
        /// `arn:aws:iam::123456789012:role/MyWebIdentityRole`.
        role_arn: ValueOrExpression<String>,

        /// Name for the assumed-role session. Appears in CloudTrail logs.
        /// Defaults to `WebIdentitySession`.
        session_name: Option<ValueOrExpression<String>>,

        /// Custom [STS](https://docs.aws.amazon.com/STS/latest/APIReference/welcome.html)
        /// endpoint for token exchange.
        ///
        /// Defaults to `https://sts.<region>.amazonaws.com`. Override when
        /// using a regional STS endpoint or a private endpoint.
        sts_endpoint: Option<ValueOrExpression<String>>,
    },

    /// [ECS task IAM role](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-iam-roles.html)
    /// credentials fetched from the ECS task metadata endpoint.
    ///
    /// The relative URI is normally injected by ECS into the
    /// `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` environment variable.
    ///
    /// ```yaml
    /// credentials:
    ///   type: ecs_task
    ///   relative_uri: /v2/credentials/abc123
    /// ```
    EcsTask {
        /// Path component of the ECS credential endpoint, e.g.
        /// `/v2/credentials/abc123`.
        ///
        /// Appended to the fixed ECS metadata base URL
        /// `http://169.254.170.2`.
        relative_uri: ValueOrExpression<String>,
    },

    /// [EKS Pod Identity](https://docs.aws.amazon.com/eks/latest/userguide/pod-identities.html)
    /// credentials, fetched from a container credential endpoint using a
    /// Kubernetes-issued token for authentication.
    ///
    /// Both `full_uri` and `token_file` are normally injected by the EKS Pod
    /// Identity agent via the `AWS_CONTAINER_CREDENTIALS_FULL_URI` and
    /// `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` environment variables.
    ///
    /// ```yaml
    /// credentials:
    ///   type: eks_pod_identity
    ///   full_uri: http://169.254.170.2/v2/credentials/abc123
    ///   token_file: /var/run/secrets/eks.amazonaws.com/serviceaccount/token
    /// ```
    EksPodIdentity {
        /// Full URL of the container credential endpoint, e.g.
        /// `http://169.254.170.2/v2/credentials/abc123`.
        full_uri: ValueOrExpression<String>,

        /// Path to the file containing the bearer token used to authenticate
        /// with the credential endpoint, e.g.
        /// `/var/run/secrets/eks.amazonaws.com/serviceaccount/token`.
        token_file: ValueOrExpression<String>,
    },

    /// EC2 [Instance Metadata Service (IMDSv2)](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html)
    /// credentials via an attached IAM instance role.
    ///
    /// This is the implicit default when `credentials` is omitted entirely.
    /// Use this variant explicitly only when you need to tune IMDSv1 fallback
    /// or override the metadata endpoint.
    ///
    /// ```yaml
    /// credentials:
    ///   type: instance_metadata
    ///   imdsv1_fallback: true
    /// ```
    InstanceMetadata {
        /// Fall back to [IMDSv1](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html)
        /// if IMDSv2 returns a 403.
        ///
        /// IMDSv1 is disabled by default because it is vulnerable to
        /// [SSRF attacks](https://aws.amazon.com/blogs/security/defense-in-depth-open-firewalls-reverse-proxies-ssrf-vulnerabilities-ec2-instance-metadata-service/).
        /// Only enable this for environments running old tooling (e.g. kube2iam
        /// versions that predate IMDSv2 support) that cannot be upgraded.
        imdsv1_fallback: Option<ValueOrExpression<bool>>,

        /// Override the IMDS endpoint URL.
        ///
        /// Defaults to the IPv4 endpoint `http://169.254.169.254`. The IPv6
        /// alternative `http://fd00:ec2::254` can be used on dual-stack
        /// instances.
        metadata_endpoint: Option<ValueOrExpression<String>>,
    },
}
