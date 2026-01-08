use hive_router_config::telemetry::tracing::OtlpGrpcTlsConfig;
use std::{collections::HashMap, str::FromStr};
use tonic::{
    metadata::{MetadataKey, MetadataMap},
    transport::ClientTlsConfig,
};
use vrl::core::Value;

use crate::{
    expressions::{CompileExpression, ExecutableProgram},
    telemetry::error::TelemetryError,
};
use hive_router_config::primitives::value_or_expression::ValueOrExpression;

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
        None => Ok(ClientTlsConfig::default()),
    }
}

pub(super) fn resolve_string_map(
    map: &HashMap<String, ValueOrExpression<String>>,
    context_prefix: &str,
) -> Result<HashMap<String, String>, TelemetryError> {
    map.iter()
        .map(|(k, v)| {
            let value = resolve_value_or_expression(v, &format!("{} '{}'", context_prefix, k))?;
            Ok((k.clone(), value))
        })
        .collect()
}

pub fn evaluate_expression_as_string(
    expression: &str,
    context: &str,
) -> Result<String, TelemetryError> {
    Ok(expression
        // compile
        .compile_expression(None)
        .map_err(|e| {
            TelemetryError::TracesExporterSetup(format!(
                "Failed to compile {} expression: {}",
                context, e
            ))
        })?
        // execute
        .execute(Value::Null) // no input context as we are in setup phase
        .map_err(|e| {
            TelemetryError::TracesExporterSetup(format!(
                "Failed to execute {} expression: {}",
                context, e
            ))
        })?
        // coerce
        .as_str()
        .ok_or_else(|| {
            TelemetryError::TracesExporterSetup(format!(
                "{} expression must return a string",
                context
            ))
        })?
        .to_string())
}

pub fn resolve_value_or_expression(
    value_or_expr: &ValueOrExpression<String>,
    context: &str,
) -> Result<String, TelemetryError> {
    match value_or_expr {
        ValueOrExpression::Value(v) => Ok(v.clone()),
        ValueOrExpression::Expression { expression } => {
            evaluate_expression_as_string(expression, context)
        }
    }
}
