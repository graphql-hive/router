use std::collections::BTreeMap;

use http::HeaderMap;
use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};

use crate::{
    execution::client_request_details::ClientRequestDetails,
    headers::{
        errors::HeaderRuleRuntimeError,
        expression::vrl_value_to_header_value,
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
) -> Result<(), HeaderRuleRuntimeError> {
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
        action.apply_request_headers(&ctx, output_headers)?;
    }

    Ok(())
}

pub struct RequestExpressionContext<'a> {
    pub subgraph_name: &'a str,
    pub client_request: &'a ClientRequestDetails<'a>,
}

trait ApplyRequestHeader {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError>;
}

impl ApplyRequestHeader for RequestHeaderRule {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
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
    ) -> Result<(), HeaderRuleRuntimeError> {
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
                    return Ok(());
                }

                output_headers.append(destination_name, default_value.clone());
            }
        }

        Ok(())
    }
}

impl ApplyRequestHeader for RequestPropagateRegex {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
        for (header_name, header_value) in ctx.client_request.headers.iter() {
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

        Ok(())
    }
}

impl ApplyRequestHeader for RequestInsertStatic {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
        if !is_denied_header(&self.name) {
            if is_never_join_header(&self.name) {
                output_headers.append(self.name.clone(), self.value.clone());
            } else {
                output_headers.insert(self.name.clone(), self.value.clone());
            }
        }

        Ok(())
    }
}

impl ApplyRequestHeader for RequestInsertExpression {
    fn apply_request_headers(
        &self,
        ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
        if is_denied_header(&self.name) {
            return Ok(());
        }

        let mut target = VrlTargetValue {
            value: ctx.into(),
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let timezone = VrlTimeZone::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);
        let value = self.expression.resolve(&mut ctx).map_err(|err| {
            HeaderRuleRuntimeError::new_expression_evaluation(self.name.to_string(), Box::new(err))
        })?;

        if let Some(header_value) = vrl_value_to_header_value(value) {
            if is_never_join_header(&self.name) {
                output_headers.append(self.name.clone(), header_value);
            } else {
                output_headers.insert(self.name.clone(), header_value);
            }
        }

        Ok(())
    }
}

impl ApplyRequestHeader for RequestRemoveNamed {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }
            output_headers.remove(header_name);
        }

        Ok(())
    }
}

impl ApplyRequestHeader for RequestRemoveRegex {
    fn apply_request_headers(
        &self,
        _ctx: &RequestExpressionContext,
        output_headers: &mut HeaderMap,
    ) -> Result<(), HeaderRuleRuntimeError> {
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

        Ok(())
    }
}
