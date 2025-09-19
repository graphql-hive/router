use crate::headers::{
    errors::HeaderRuleCompileError,
    plan::{
        HeaderAggregationStrategy, HeaderRulesPlan, RequestHeaderRule, RequestHeaderRules,
        RequestInsertStatic, RequestPropagateNamed, RequestPropagateRegex, RequestRemoveNamed,
        RequestRemoveRegex, ResponseHeaderRule, ResponseHeaderRules, ResponseInsertStatic,
        ResponsePropagateNamed, ResponsePropagateRegex, ResponseRemoveNamed, ResponseRemoveRegex,
    },
};

use hive_router_config::headers as config;
use http::HeaderName;
use regex_automata::{meta, util::syntax::Config as SyntaxConfig};

pub trait HeaderRuleCompiler<A> {
    fn compile(&self, actions: &mut A) -> Result<(), HeaderRuleCompileError>;
}

impl HeaderRuleCompiler<Vec<RequestHeaderRule>> for config::RequestHeaderRule {
    fn compile(&self, actions: &mut Vec<RequestHeaderRule>) -> Result<(), HeaderRuleCompileError> {
        match self {
            config::RequestHeaderRule::Propagate(rule) => {
                let spec = materialize_match_spec(
                    &rule.spec,
                    rule.rename.as_ref(),
                    rule.default.as_ref(),
                )?;

                if !spec.header_names.is_empty() {
                    actions.push(RequestHeaderRule::PropagateNamed(RequestPropagateNamed {
                        names: spec.header_names,
                        default: spec.default_header_value,
                        rename: spec.rename_header,
                    }));
                }
                if spec.include_regex.is_some() {
                    actions.push(RequestHeaderRule::PropagateRegex(RequestPropagateRegex {
                        include: spec.include_regex,
                        exclude: spec.exclude_regex,
                    }));
                }
            }
            config::RequestHeaderRule::Insert(rule) => {
                let config::InsertSource::Value { value } = &rule.source;
                actions.push(RequestHeaderRule::InsertStatic(RequestInsertStatic {
                    name: build_header_name(&rule.name)?,
                    value: build_header_value(&rule.name, value)?,
                }));
            }
            config::RequestHeaderRule::Remove(rule) => {
                let spec = materialize_match_spec(&rule.spec, None, None)?;
                if !spec.header_names.is_empty() {
                    actions.push(RequestHeaderRule::RemoveNamed(RequestRemoveNamed {
                        names: spec.header_names,
                    }));
                }
                if let Some(regex_set) = spec.include_regex {
                    actions.push(RequestHeaderRule::RemoveRegex(RequestRemoveRegex {
                        regex: regex_set,
                    }));
                }
            }
        }

        Ok(())
    }
}

impl HeaderRuleCompiler<Vec<ResponseHeaderRule>> for config::ResponseHeaderRule {
    fn compile(&self, actions: &mut Vec<ResponseHeaderRule>) -> Result<(), HeaderRuleCompileError> {
        match self {
            config::ResponseHeaderRule::Propagate(rule) => {
                let aggregation_strategy =
                    match rule.algorithm.unwrap_or(config::AggregationAlgo::Last) {
                        config::AggregationAlgo::First => HeaderAggregationStrategy::First,
                        config::AggregationAlgo::Last => HeaderAggregationStrategy::Last,
                        config::AggregationAlgo::Append => HeaderAggregationStrategy::Append,
                    };
                let spec = materialize_match_spec(
                    &rule.spec,
                    rule.rename.as_ref(),
                    rule.default.as_ref(),
                )?;

                if !spec.header_names.is_empty() {
                    actions.push(ResponseHeaderRule::PropagateNamed(ResponsePropagateNamed {
                        names: spec.header_names,
                        rename: spec.rename_header,
                        default: spec.default_header_value,
                        strategy: aggregation_strategy,
                    }));
                }

                if spec.include_regex.is_some() || spec.exclude_regex.is_some() {
                    actions.push(ResponseHeaderRule::PropagateRegex(ResponsePropagateRegex {
                        include: spec.include_regex,
                        exclude: spec.exclude_regex,
                        strategy: aggregation_strategy,
                    }));
                }
            }
            config::ResponseHeaderRule::Insert(rule) => {
                let config::InsertSource::Value { value } = &rule.source;
                actions.push(ResponseHeaderRule::InsertStatic(ResponseInsertStatic {
                    name: build_header_name(&rule.name)?,
                    value: build_header_value(&rule.name, value)?,
                }));
            }
            config::ResponseHeaderRule::Remove(rule) => {
                let spec = materialize_match_spec(&rule.spec, None, None)?;
                if !spec.header_names.is_empty() {
                    actions.push(ResponseHeaderRule::RemoveNamed(ResponseRemoveNamed {
                        names: spec.header_names,
                    }));
                }
                if let Some(regex_set) = spec.include_regex {
                    actions.push(ResponseHeaderRule::RemoveRegex(ResponseRemoveRegex {
                        regex: regex_set,
                    }));
                }
            }
        }

        Ok(())
    }
}

pub fn compile_headers_plan(
    cfg: &config::HeadersConfig,
) -> Result<HeaderRulesPlan, HeaderRuleCompileError> {
    let mut request_plan = RequestHeaderRules::default();
    let mut response_plan = ResponseHeaderRules::default();

    if let Some(global_rules) = &cfg.all {
        request_plan.global = compile_request_header_rules(global_rules)?;
        response_plan.global = compile_response_header_rules(global_rules)?;
    }

    if let Some(subgraph_rules_map) = &cfg.subgraphs {
        for (subgraph_name, subgraph_rules) in subgraph_rules_map {
            let request_actions = compile_request_header_rules(subgraph_rules)?;
            let response_actions = compile_response_header_rules(subgraph_rules)?;
            request_plan
                .by_subgraph
                .insert(subgraph_name.clone(), request_actions);
            response_plan
                .by_subgraph
                .insert(subgraph_name.clone(), response_actions);
        }
    }

    Ok(HeaderRulesPlan {
        request: request_plan,
        response: response_plan,
    })
}

fn compile_request_header_rules(
    header_rules: &config::HeaderRules,
) -> Result<Vec<RequestHeaderRule>, HeaderRuleCompileError> {
    let mut request_actions = Vec::new();
    if let Some(request_rule_entries) = &header_rules.request {
        for request_rule in request_rule_entries {
            request_rule.compile(&mut request_actions)?;
        }
    }
    Ok(request_actions)
}

fn compile_response_header_rules(
    header_rules: &config::HeaderRules,
) -> Result<Vec<ResponseHeaderRule>, HeaderRuleCompileError> {
    let mut response_actions = Vec::new();
    if let Some(response_rule_entries) = &header_rules.response {
        for response_rule in response_rule_entries {
            response_rule.compile(&mut response_actions)?;
        }
    }
    Ok(response_actions)
}

struct HeaderMatchSpecResult {
    header_names: Vec<HeaderName>,
    include_regex: Option<meta::Regex>,
    exclude_regex: Option<meta::Regex>,
    rename_header: Option<HeaderName>,
    default_header_value: Option<http::HeaderValue>,
}

fn materialize_match_spec(
    match_spec: &config::MatchSpec,
    rename_to: Option<&String>,
    default_value: Option<&String>,
) -> Result<HeaderMatchSpecResult, HeaderRuleCompileError> {
    let header_names = match &match_spec.named {
        Some(config::OneOrMany::One(single_name)) => vec![build_header_name(single_name)?],
        Some(config::OneOrMany::Many(many_names)) => many_names
            .iter()
            .map(|name| build_header_name(name))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    let include_regex = match match_spec.matching.as_ref() {
        None => None,
        Some(config::OneOrMany::One(pattern)) => build_regex_many(std::slice::from_ref(pattern))?,
        Some(config::OneOrMany::Many(pattern_vec)) => build_regex_many(pattern_vec)?,
    };

    let exclude_regex = match match_spec.exclude.as_deref() {
        None => None,
        Some(pattern_vec) => build_regex_many(pattern_vec)?,
    };

    let rename_header = rename_to
        .map(|name| match header_names.len() == 1 {
            true => build_header_name(name),
            false => Err(HeaderRuleCompileError::InvalidRename),
        })
        .transpose()?;

    let default_header_value = default_value
        .map(|value| match header_names.len() == 1 {
            true => build_header_value(header_names[0].as_str(), value),
            false => Err(HeaderRuleCompileError::InvalidDefault),
        })
        .transpose()?;

    Ok(HeaderMatchSpecResult {
        header_names,
        include_regex,
        exclude_regex,
        rename_header,
        default_header_value,
    })
}

fn build_header_name(header_name_str: &str) -> Result<http::HeaderName, HeaderRuleCompileError> {
    http::HeaderName::from_bytes(header_name_str.as_bytes())
        .map_err(|err| HeaderRuleCompileError::BadHeaderName(header_name_str.into(), err))
}

fn build_header_value(
    header_name_str: &str,
    header_value_str: &str,
) -> Result<http::HeaderValue, HeaderRuleCompileError> {
    http::HeaderValue::from_str(header_value_str)
        .map_err(|err| HeaderRuleCompileError::BadHeaderValue(header_name_str.to_string(), err))
}

fn build_regex_many(patterns: &[String]) -> Result<Option<meta::Regex>, HeaderRuleCompileError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut regex_builder = meta::Regex::builder();
    regex_builder.syntax(SyntaxConfig::new().unicode(false).utf8(false));
    regex_builder
        .build_many(patterns)
        .map(Some)
        .map_err(|e| Box::new(e).into())
}

#[cfg(test)]
mod tests {
    use hive_router_config::headers as config;
    use http::HeaderName;

    use crate::headers::{
        compile::{build_header_value, HeaderRuleCompiler},
        errors::HeaderRuleCompileError,
        plan::{HeaderAggregationStrategy, RequestHeaderRule, ResponseHeaderRule},
    };

    fn header_name_owned(s: &str) -> HeaderName {
        HeaderName::from_bytes(s.as_bytes()).unwrap()
    }

    #[test]
    fn test_propagate_named_request() {
        let rule = config::RequestHeaderRule::Propagate(config::RequestPropagateRule {
            spec: config::MatchSpec {
                named: Some(config::OneOrMany::One("x-test".to_string())),
                matching: None,
                exclude: None,
            },
            rename: None,
            default: None,
        });
        let mut actions = Vec::new();
        rule.compile(&mut actions).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            RequestHeaderRule::PropagateNamed(data) => {
                assert_eq!(data.names, vec![header_name_owned("x-test")]);
                assert!(data.default.is_none());
                assert!(data.rename.is_none());
            }
            _ => panic!("Expected PropagateNamed"),
        }
    }

    #[test]
    fn test_set_request() {
        let rule = config::RequestHeaderRule::Insert(config::InsertRule {
            name: "x-set".to_string(),
            source: config::InsertSource::Value {
                value: "abc".to_string(),
            },
        });
        let mut actions = Vec::new();
        rule.compile(&mut actions).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            RequestHeaderRule::InsertStatic(data) => {
                assert_eq!(data.name, header_name_owned("x-set"));
                assert_eq!(data.value, build_header_value("x-set", "abc").unwrap());
            }
            _ => panic!("Expected SetStatic"),
        }
    }

    #[test]
    fn test_remove_named_request() {
        let rule = config::RequestHeaderRule::Remove(config::RemoveRule {
            spec: config::MatchSpec {
                named: Some(config::OneOrMany::One("x-remove".to_string())),
                matching: None,
                exclude: None,
            },
        });
        let mut actions = Vec::new();
        rule.compile(&mut actions).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            RequestHeaderRule::RemoveNamed(data) => {
                assert_eq!(data.names, vec![header_name_owned("x-remove")]);
            }
            _ => panic!("Expected RemoveNamed"),
        }
    }

    #[test]
    fn test_invalid_default_request() {
        let rule = config::RequestHeaderRule::Propagate(config::RequestPropagateRule {
            spec: config::MatchSpec {
                named: Some(config::OneOrMany::Many(vec![
                    "x1".to_string(),
                    "x2".to_string(),
                ])),
                matching: None,
                exclude: None,
            },
            rename: None,
            default: Some("def".to_string()),
        });
        let mut actions = Vec::new();
        let err = rule.compile(&mut actions).unwrap_err();
        match err {
            HeaderRuleCompileError::InvalidDefault => {}
            _ => panic!("Expected InvalidDefault error"),
        }
    }

    #[test]
    fn test_propagate_named_response() {
        let rule = config::ResponseHeaderRule::Propagate(config::ResponsePropagateRule {
            spec: config::MatchSpec {
                named: Some(config::OneOrMany::One("x-resp".to_string())),
                matching: None,
                exclude: None,
            },
            rename: None,
            default: None,
            algorithm: Some(config::AggregationAlgo::First),
        });
        let mut actions = Vec::new();
        rule.compile(&mut actions).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ResponseHeaderRule::PropagateNamed(data) => {
                assert_eq!(data.names, vec![header_name_owned("x-resp")]);
                assert!(matches!(data.strategy, HeaderAggregationStrategy::First));
            }
            _ => panic!("Expected PropagateNamed"),
        }
    }
}
