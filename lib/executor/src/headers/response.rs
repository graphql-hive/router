use std::{collections::BTreeMap, iter::once};

use crate::{
    execution::client_request_details::ClientRequestDetails,
    headers::{
        errors::HeaderRuleRuntimeError,
        expression::vrl_value_to_header_value,
        plan::{
            HeaderAggregationStrategy, HeaderRulesPlan, ResponseHeaderAggregator,
            ResponseHeaderRule, ResponseInsertExpression, ResponseInsertStatic,
            ResponsePropagateNamed, ResponsePropagateRegex, ResponseRemoveNamed,
            ResponseRemoveRegex,
        },
        sanitizer::is_denied_header,
    },
};

use super::sanitizer::is_never_join_header;
use http::{header::InvalidHeaderValue, HeaderMap, HeaderName, HeaderValue};
use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};

pub fn apply_subgraph_response_headers(
    header_rule_plan: &HeaderRulesPlan,
    subgraph_name: &str,
    subgraph_headers: &HeaderMap,
    client_request_details: &ClientRequestDetails,
    accumulator: &mut ResponseHeaderAggregator,
) -> Result<(), HeaderRuleRuntimeError> {
    let global_actions = &header_rule_plan.response.global;
    let subgraph_actions = header_rule_plan.response.by_subgraph.get(subgraph_name);

    let ctx = ResponseExpressionContext {
        subgraph_name,
        subgraph_headers,
        client_request: client_request_details,
    };

    for action in global_actions
        .iter()
        .chain(subgraph_actions.into_iter().flatten())
    {
        action.apply_response_headers(&ctx, accumulator)?;
    }

    Ok(())
}

pub struct ResponseExpressionContext<'a> {
    pub subgraph_name: &'a str,
    pub client_request: &'a ClientRequestDetails<'a>,
    pub subgraph_headers: &'a HeaderMap,
}

trait ApplyResponseHeader {
    fn apply_response_headers(
        &self,
        ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError>;
}

impl ApplyResponseHeader for ResponseHeaderRule {
    fn apply_response_headers(
        &self,
        ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        match self {
            ResponseHeaderRule::PropagateNamed(data) => {
                data.apply_response_headers(ctx, accumulator)
            }
            ResponseHeaderRule::PropagateRegex(data) => {
                data.apply_response_headers(ctx, accumulator)
            }
            ResponseHeaderRule::InsertStatic(data) => data.apply_response_headers(ctx, accumulator),
            ResponseHeaderRule::InsertExpression(data) => {
                data.apply_response_headers(ctx, accumulator)
            }
            ResponseHeaderRule::RemoveNamed(data) => data.apply_response_headers(ctx, accumulator),
            ResponseHeaderRule::RemoveRegex(data) => data.apply_response_headers(ctx, accumulator),
        }
    }
}

impl ApplyResponseHeader for ResponsePropagateNamed {
    fn apply_response_headers(
        &self,
        ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        let mut matched = false;

        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }

            if let Some(header_value) = ctx.subgraph_headers.get(header_name) {
                matched = true;
                write_agg(
                    accumulator,
                    self.rename.as_ref().unwrap_or(header_name),
                    header_value,
                    self.strategy,
                );
            }
        }

        if !matched {
            if let (Some(default_value), Some(first_name)) = (&self.default, self.names.first()) {
                let destination_name = self.rename.as_ref().unwrap_or(first_name);

                if is_denied_header(destination_name) {
                    return Ok(());
                }

                write_agg(accumulator, destination_name, default_value, self.strategy);
            }
        }

        Ok(())
    }
}

impl ApplyResponseHeader for ResponsePropagateRegex {
    fn apply_response_headers(
        &self,
        ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        for (header_name, header_value) in ctx.subgraph_headers {
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

            write_agg(accumulator, header_name, header_value, self.strategy);
        }

        Ok(())
    }
}

impl ApplyResponseHeader for ResponseInsertStatic {
    fn apply_response_headers(
        &self,
        _ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        if is_denied_header(&self.name) {
            return Ok(());
        }

        let strategy = if is_never_join_header(&self.name) {
            HeaderAggregationStrategy::Append
        } else {
            self.strategy
        };

        write_agg(accumulator, &self.name, &self.value, strategy);

        Ok(())
    }
}

impl ApplyResponseHeader for ResponseInsertExpression {
    fn apply_response_headers(
        &self,
        ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
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
            HeaderRuleRuntimeError::ExpressionEvaluation(self.name.to_string(), Box::new(err))
        })?;

        if let Some(header_value) = vrl_value_to_header_value(value) {
            let strategy = if is_never_join_header(&self.name) {
                HeaderAggregationStrategy::Append
            } else {
                self.strategy
            };

            write_agg(accumulator, &self.name, &header_value, strategy);
        }

        Ok(())
    }
}

impl ApplyResponseHeader for ResponseRemoveNamed {
    fn apply_response_headers(
        &self,
        _ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }
            accumulator.entries.remove(header_name);
        }

        Ok(())
    }
}

impl ApplyResponseHeader for ResponseRemoveRegex {
    fn apply_response_headers(
        &self,
        _ctx: &ResponseExpressionContext,
        accumulator: &mut ResponseHeaderAggregator,
    ) -> Result<(), HeaderRuleRuntimeError> {
        accumulator.entries.retain(|name, _| {
            if is_denied_header(name) {
                // Denied headers (hop-by–hop) are never inserted in the first place
                // and should not be removed here.
                return true;
            }

            !self.regex.is_match(name.as_str().as_bytes())
        });

        Ok(())
    }
}

/// Write a header to the aggregator according to the specified strategy.
fn write_agg(
    agg: &mut ResponseHeaderAggregator,
    name: &HeaderName,
    value: &HeaderValue,
    strategy: HeaderAggregationStrategy,
) {
    let strategy = if is_never_join_header(name) {
        HeaderAggregationStrategy::Append
    } else {
        strategy
    };

    if !agg.entries.contains_key(name) {
        agg.entries
            .insert(name.clone(), (strategy, once(value.clone()).collect()));
        return;
    }

    // The `expect` is safe because we just inserted the entry if it didn't exist
    let (strategy, values) = agg.entries.get_mut(name).expect("Expected entry to exist");

    match (strategy, values.len()) {
        (HeaderAggregationStrategy::First, 0) => {
            values.push(value.clone());
        }
        (HeaderAggregationStrategy::Last, _) => {
            values.clear();
            values.push(value.clone());
        }
        (HeaderAggregationStrategy::Append, _) => {
            values.push(value.clone());
        }
        (_, _) => {}
    }
}

/// Modify the outgoing client response headers based on the aggregated headers from subgraphs.
pub fn modify_client_response_headers(
    agg: ResponseHeaderAggregator,
    out: &mut HeaderMap,
) -> Result<(), HeaderRuleRuntimeError> {
    for (name, (agg_strategy, mut values)) in agg.entries {
        if values.is_empty() {
            continue;
        }

        if is_never_join_header(&name) {
            // never-join headers must be emitted as multiple header fields
            for value in values {
                out.append(name.clone(), value);
            }
            continue;
        }

        if values.len() == 1 {
            out.insert(name, values.pop().unwrap());
            continue;
        }

        if matches!(agg_strategy, HeaderAggregationStrategy::Append) {
            let joined = join_with_comma(&values)
                .map_err(|_| HeaderRuleRuntimeError::BadHeaderValue(name.to_string()))?;
            out.insert(name, joined);
        }
    }

    Ok(())
}

#[inline]
fn join_with_comma(values: &[HeaderValue]) -> Result<HeaderValue, InvalidHeaderValue> {
    // Compute capacity: sum of lengths + ", ".len() * (n-1)
    let mut cap = 0usize;

    for value in values {
        cap += value.as_bytes().len();
    }

    if values.len() > 1 {
        cap += 2 * (values.len() - 1);
    }

    let mut buf = Vec::with_capacity(cap);
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            buf.extend_from_slice(b", ");
        }
        buf.extend_from_slice(value.as_bytes());
    }
    HeaderValue::from_bytes(&buf)
}
