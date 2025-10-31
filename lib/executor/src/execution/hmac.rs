use std::collections::BTreeMap;

use bytes::{BufMut, Bytes};
use hive_router_config::hmac_signature::{BooleanOrExpression, HMACSignatureConfig};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

use crate::{
    execution::client_request_details::ClientRequestDetails,
    executors::{error::SubgraphExecutorError, http::FIRST_EXTENSION_STR},
    utils::{
        consts::{CLOSE_BRACE, COLON, COMMA, QUOTE},
        expression::{compile_expression, execute_expression_with_value},
    },
};

#[derive(Debug)]
pub enum BooleanOrProgram {
    Boolean(bool),
    Program(Box<VrlProgram>),
}

pub fn compile_hmac_config(
    config: &HMACSignatureConfig,
) -> Result<BooleanOrProgram, SubgraphExecutorError> {
    match &config.enabled {
        BooleanOrExpression::Boolean(b) => Ok(BooleanOrProgram::Boolean(*b)),
        BooleanOrExpression::Expression { expression } => {
            let program = compile_expression(expression, None)
                .map_err(SubgraphExecutorError::HMACExpressionBuild)?;
            Ok(BooleanOrProgram::Program(Box::new(program)))
        }
    }
}
type HmacSha256 = Hmac<Sha256>;

pub fn sign_hmac(
    hmac_program: &BooleanOrProgram,
    hmac_config: &HMACSignatureConfig,
    subgraph_name: &str,
    client_request: &ClientRequestDetails,
    first_extension: &mut bool,
    body: &mut Vec<u8>,
) -> Result<(), SubgraphExecutorError> {
    let should_sign_hmac = match &hmac_program {
        BooleanOrProgram::Boolean(b) => *b,
        BooleanOrProgram::Program(expr) => {
            // .subgraph
            let subgraph_value = VrlValue::Object(BTreeMap::from([(
                "name".into(),
                VrlValue::Bytes(Bytes::from(subgraph_name.to_owned())),
            )]));
            // .request
            let request_value: VrlValue = client_request.into();
            let target_value = VrlValue::Object(BTreeMap::from([
                ("subgraph".into(), subgraph_value),
                ("request".into(), request_value),
            ]));
            let result = execute_expression_with_value(expr, target_value);
            match result {
                Ok(VrlValue::Boolean(b)) => b,
                Ok(_) => {
                    return Err(SubgraphExecutorError::HMACSignatureError(
                        "HMAC signature expression did not evaluate to a boolean".to_string(),
                    ));
                }
                Err(e) => {
                    return Err(SubgraphExecutorError::HMACSignatureError(format!(
                        "HMAC signature expression evaluation error: {}",
                        e
                    )));
                }
            }
        }
    };

    if should_sign_hmac {
        if hmac_config.secret.is_empty() {
            return Err(SubgraphExecutorError::HMACSignatureError(
                "HMAC signature secret is empty".to_string(),
            ));
        }
        let mut mac = HmacSha256::new_from_slice(hmac_config.secret.as_bytes()).map_err(|e| {
            SubgraphExecutorError::HMACSignatureError(format!(
                "Failed to create HMAC instance: {}",
                e
            ))
        })?;
        let mut body_without_extensions = body.clone();
        body_without_extensions.put(CLOSE_BRACE);
        mac.update(&body_without_extensions);
        let result = mac.finalize();
        let result_bytes = result.into_bytes();
        if *first_extension {
            body.put(FIRST_EXTENSION_STR);
            *first_extension = false;
        } else {
            body.put(COMMA);
        }
        body.put(QUOTE);
        body.put(hmac_config.extension_name.as_bytes());
        body.put(QUOTE);
        body.put(COLON);
        let hmac_hex = hex::encode(result_bytes);
        body.put(QUOTE);
        body.put(hmac_hex.as_bytes());
        body.put(QUOTE);
    }
    Ok(())
}
