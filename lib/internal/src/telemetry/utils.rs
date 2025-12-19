use hive_router_config::telemetry::tracing::OtlpGrpcTlsConfig;
use std::{collections::HashMap, str::FromStr};
use tonic::{
    metadata::{MetadataKey, MetadataMap},
    transport::ClientTlsConfig,
};

use crate::telemetry::error::TelemetryError;

pub(super) fn build_metadata(
    headers: HashMap<String, String>,
) -> Result<MetadataMap, TelemetryError> {
    let metadata = tonic::metadata::MetadataMap::with_capacity(headers.len());

    headers
        .into_iter()
        .try_fold(metadata, |mut acc, (header_name, header_value)| {
            let key = MetadataKey::from_str(header_name.as_str()).map_err(|e| {
                TelemetryError::Internal(format!("Invalid metadata key '{}': {}", header_name, e))
            })?;
            acc.insert(
                key,
                header_value.as_str().parse().map_err(|e| {
                    TelemetryError::Internal(format!(
                        "Invalid metadata value for key '{}': {}",
                        header_name, e
                    ))
                })?,
            );
            Ok(acc)
        })
}

pub(super) fn build_tls_config(
    tls: Option<&OtlpGrpcTlsConfig>,
) -> Result<ClientTlsConfig, TelemetryError> {
    match tls {
        Some(tls_config) => ClientTlsConfig::try_from(tls_config)
            .map_err(|e| TelemetryError::TracesExporterSetup(e.to_string())),
        None => Ok(ClientTlsConfig::default().with_native_roots()),
    }
}
