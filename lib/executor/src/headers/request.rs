use std::collections::BTreeMap;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue};
use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};

use crate::{
    execution::plan::ClientRequestDetails,
    headers::{
        plan::{
            HeaderRulesPlan, RequestHeaderRule, RequestInsertExpression, RequestInsertStatic,
            RequestPropagateNamed, RequestPropagateRegex, RequestRemoveNamed, RequestRemoveRegex,
        },
        sanitizer::{is_denied_header, is_never_join_header},
    },
};

pub fn modify_subgraph_request_headers(
    header_rule_plan: &HeaderRulesPlan,
    subgraph_name: &str,
    client_request: &ClientRequestDetails,
    output_headers: &mut HeaderMap,
) {
    let global_actions = &header_rule_plan.request.global;
    let subgraph_actions = header_rule_plan.request.by_subgraph.get(subgraph_name);

    let ctx = RequestExpressionContext {
        subgraph_name,
        client_request,
    };

    for action in global_actions
        .iter()
        .chain(subgraph_actions.into_iter().flatten())
    {
        action.apply_request_headers(&ctx, output_headers);
    }
}

pub struct RequestExpressionContext<'a> {
    subgraph_name: &'a str,
    client_request: &'a ClientRequestDetails<'a>,
}

trait ApplyRequestHeader {
    fn apply_request_headers(&self, ctx: &RequestExpressionContext, output_headers: &mut HeaderMap);
}

impl ApplyRequestHeader for RequestHeaderRule {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        match self {
            Self::PropagateNamed(data) => data.apply_request_headers(ctx, output_headers),
            Self::PropagateRegex(data) => data.apply_request_headers(ctx, output_headers),
            Self::InsertStatic(data) => data.apply_request_headers(ctx, output_headers),
            Self::InsertExpression(data) => data.apply_request_headers(ctx, output_headers),
            Self::RemoveNamed(data) => data.apply_request_headers(ctx, output_headers),
            Self::RemoveRegex(data) => data.apply_request_headers(ctx, output_headers),
        }
    }
}

impl ApplyRequestHeader for RequestPropagateNamed {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        let mut matched = false;

        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }
            if let Some(header_value) = ctx.client_request.headers.get(header_name) {
                let destination_name = self.rename.as_ref().unwrap_or(header_name);
                output_headers.append(destination_name, header_value.into());
                matched = true;
            }
        }

        if !matched {
            // If no headers matched, and a default is provided, use it
            if let (Some(default_value), Some(first_name)) = (&self.default, self.names.first()) {
                let destination_name = self.rename.as_ref().unwrap_or(first_name);

                if is_denied_header(destination_name) {
                    return;
                }

                output_headers.append(destination_name, default_value.clone());
            }
        }
    }
}

impl ApplyRequestHeader for RequestPropagateRegex {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        for (header_name, header_value) in ctx.client_request.headers {
            if is_denied_header(header_name) {
                continue;
            }

            let header_bytes = header_name.as_str().as_bytes();

            let Some(include_regex) = &self.include else {
                continue;
            };

            if !include_regex.is_match(header_bytes) {
                continue;
            }

            if self
                .exclude
                .as_ref()
                .is_some_and(|regex| regex.is_match(header_bytes))
            {
                continue;
            }

            output_headers.append(header_name, header_value.into());
        }
    }
}

impl ApplyRequestHeader for RequestInsertStatic {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        if !is_denied_header(&self.name) {
            if is_never_join_header(&self.name) {
                output_headers.append(self.name.clone(), self.value.clone());
            } else {
                output_headers.insert(self.name.clone(), self.value.clone());
            }
        }
    }
}

impl ApplyRequestHeader for RequestInsertExpression {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        if !is_denied_header(&self.name) {
            let subgraph_value = {
                let value = VrlValue::Bytes(Bytes::from(ctx.subgraph_name.to_owned()));
                VrlValue::Object(BTreeMap::from([("name".into(), value)]))
            };

            let headers_value = {
                let mut obj = BTreeMap::new();
                for (header_name, header_value) in ctx.client_request.headers.iter() {
                    match header_value.to_str() {
                        Ok(value) => {
                            obj.insert(
                                header_name.as_str().into(),
                                VrlValue::Bytes(Bytes::from(value.to_owned())),
                            );
                        }
                        Err(_) => continue,
                    }
                }

                VrlValue::Object(obj)
            };

            let url_value = VrlValue::Object(BTreeMap::from([
                (
                    "host".into(),
                    ctx.client_request.url.host().unwrap_or("unknown").into(),
                ),
                ("path".into(), ctx.client_request.url.path().into()),
                (
                    "port".into(),
                    ctx.client_request.url.port_u16().unwrap_or(80).into(),
                ),
            ]));

            let operation_value = VrlValue::Object(BTreeMap::from([
                (
                    "name".into(),
                    ctx.client_request.operation.name.clone().into(),
                ),
                ("type".into(), ctx.client_request.operation.kind.into()),
            ]));

            let request_value = VrlValue::Object(BTreeMap::from([
                ("method".into(), ctx.client_request.method.as_str().into()),
                ("headers".into(), headers_value),
                ("url".into(), url_value),
                ("operation".into(), operation_value),
            ]));

            let value = VrlValue::Object(BTreeMap::from([
                ("subgraph".into(), subgraph_value),
                ("request".into(), request_value),
            ]));

            let mut target = VrlTargetValue {
                value,
                metadata: VrlValue::Object(BTreeMap::new()),
                secrets: VrlSecrets::default(),
            };

            let mut state = VrlState::default();
            let timezone = VrlTimeZone::default();
            let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);

            let value = self.expression.resolve(&mut ctx).unwrap();
            if let Some(header_value) = vrl_value_to_header_value(value) {
                if is_never_join_header(&self.name) {
                    output_headers.append(self.name.clone(), header_value);
                } else {
                    output_headers.insert(self.name.clone(), header_value);
                }
            }
        }
    }
}

pub fn vrl_value_to_header_value(value: VrlValue) -> Option<HeaderValue> {
    match value {
        VrlValue::Bytes(bytes) => HeaderValue::from_bytes(&bytes).ok(),
        VrlValue::Float(f) => HeaderValue::from_str(&f.to_string()).ok(),
        VrlValue::Boolean(b) => HeaderValue::from_str(if b { "true" } else { "false" }).ok(),
        VrlValue::Integer(i) => HeaderValue::from_str(&i.to_string()).ok(),
        _ => None,
    }
}

impl ApplyRequestHeader for RequestRemoveNamed {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }
            output_headers.remove(header_name);
        }
    }
}

impl ApplyRequestHeader for RequestRemoveRegex {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) {
        let mut headers_to_remove = Vec::new();
        for header_name in output_headers.keys() {
            if is_denied_header(header_name) {
                continue;
            }

            if self.regex.is_match(header_name.as_str().as_bytes()) {
                headers_to_remove.push(header_name.clone());
            }
        }

        for header_name in headers_to_remove.iter() {
            output_headers.remove(header_name);
        }
    }
}
