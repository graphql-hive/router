use hive_router_config::telemetry::ClientIdentificationConfig;
use hive_router_plan_executor::request_context::{RequestContextError, SharedRequestContext};
use ntex::http::HeaderMap;

pub struct ClientIdentity {
    pub(crate) name: Option<String>,
    pub(crate) version: Option<String>,
}

pub fn identify_client(
    headers: &HeaderMap,
    request_context: &SharedRequestContext,
    config: &ClientIdentificationConfig,
) -> Result<ClientIdentity, RequestContextError> {
    let mut client_name: Option<String> = None;
    let mut client_version: Option<String> = None;

    request_context.update(|ctx| {
        // telemetry.client_name takes precedence over the name header
        match &ctx.telemetry.client_name {
            Some(name) => {
                client_name = Some(name.clone());
            }
            None => {
                if let Some(name) = headers
                    .get(&config.name_header)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                {
                    ctx.telemetry.client_name = Some(name.clone());
                    client_name = Some(name);
                }
            }
        }

        // telemetry.client_version takes precedence over the version header
        match &ctx.telemetry.client_version {
            Some(version) => {
                client_version = Some(version.clone());
            }
            None => {
                if let Some(version) = headers
                    .get(&config.version_header)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                {
                    ctx.telemetry.client_version = Some(version.clone());
                    client_version = Some(version);
                }
            }
        }
    })?;

    Ok(ClientIdentity {
        name: client_name,
        version: client_version,
    })
}
