use std::collections::BTreeMap;

use hive_router_plan_executor::execution::client_request_details::{
    client_header_map_to_vrl_value, client_path_params_to_vrl_value, client_url_to_vrl_value,
    JwtRequestDetails,
};
use ntex::web::HttpRequest;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use vrl::core::Value as VrlValue;

use crate::pipeline::execution_request::ExecutionRequest;

pub fn get_expression_input_val(
    execution_request: &ExecutionRequest,
    req: &HttpRequest,
    jwt_request_details: &JwtRequestDetails<'_>,
) -> VrlValue {
    let headers_value = client_header_map_to_vrl_value(req.headers());
    let url_value = client_url_to_vrl_value(req.uri());
    let path_params_value = client_path_params_to_vrl_value(req.match_info());
    let request_obj = VrlValue::Object(BTreeMap::from([
        ("method".into(), req.method().as_str().into()),
        ("headers".into(), headers_value),
        ("url".into(), url_value),
        ("path_params".into(), path_params_value),
        ("jwt".into(), jwt_request_details.into()),
        (
            "body".into(),
            execution_request_to_vrl_value(execution_request),
        ),
    ]));

    VrlValue::Object(BTreeMap::from([("request".into(), request_obj)]))
}

fn execution_request_to_vrl_value(execution_request: &ExecutionRequest) -> VrlValue {
    let mut obj = BTreeMap::new();
    if let Some(op_name) = &execution_request.operation_name {
        obj.insert("operationName".into(), op_name.clone().into());
    }
    if let Some(query) = &execution_request.query {
        obj.insert("query".into(), query.clone().into());
    }
    for (k, v) in &execution_request.extra_params {
        obj.insert(k.clone().into(), from_sonic_value_to_vrl_value(v));
    }
    VrlValue::Object(obj)
}

fn from_sonic_value_to_vrl_value(value: &sonic_rs::Value) -> VrlValue {
    match value.get_type() {
        sonic_rs::JsonType::Null => VrlValue::Null,
        sonic_rs::JsonType::Boolean => VrlValue::Boolean(value.as_bool().unwrap_or(false)),
        sonic_rs::JsonType::Number => {
            if let Some(n) = value.as_i64() {
                VrlValue::Integer(n)
            } else if let Some(n) = value.as_f64() {
                VrlValue::from_f64_or_zero(n)
            } else {
                VrlValue::Null
            }
        }
        sonic_rs::JsonType::String => {
            if let Some(s) = value.as_str() {
                s.into()
            } else {
                VrlValue::Null
            }
        }
        sonic_rs::JsonType::Array => {
            if let Some(array) = value.as_array() {
                let vec = array.iter().map(from_sonic_value_to_vrl_value).collect();
                VrlValue::Array(vec)
            } else {
                VrlValue::Null
            }
        }
        sonic_rs::JsonType::Object => {
            if let Some(obj) = value.as_object() {
                obj.iter()
                    .map(|(k, v)| (k.into(), from_sonic_value_to_vrl_value(v)))
                    .collect::<BTreeMap<_, _>>()
                    .into()
            } else {
                VrlValue::Null
            }
        }
    }
}
