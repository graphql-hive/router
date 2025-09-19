use ahash::HashMap;
use http::{HeaderName, HeaderValue};
use regex_automata::meta::Regex;

#[derive(Clone)]
pub struct HeaderRulesPlan {
    pub request: RequestHeaderRules,
    pub response: ResponseHeaderRules,
}

type SubgraphName = String;

#[derive(Clone, Default)]
pub struct RequestHeaderRules {
    pub global: Vec<RequestHeaderRule>,
    pub by_subgraph: HashMap<SubgraphName, Vec<RequestHeaderRule>>,
}

#[derive(Clone, Default)]
pub struct ResponseHeaderRules {
    pub global: Vec<ResponseHeaderRule>,
    pub by_subgraph: HashMap<SubgraphName, Vec<ResponseHeaderRule>>,
}

#[derive(Clone)]
pub enum RequestHeaderRule {
    PropagateNamed(RequestPropagateNamed),
    PropagateRegex(RequestPropagateRegex),
    InsertStatic(RequestInsertStatic),
    RemoveNamed(RequestRemoveNamed),
    RemoveRegex(RequestRemoveRegex),
}

#[derive(Clone)]
pub struct RequestPropagateNamed {
    pub names: Vec<HeaderName>,
    pub default: Option<HeaderValue>,
    pub rename: Option<HeaderName>,
}

#[derive(Clone)]
pub struct RequestPropagateRegex {
    pub include: Option<Regex>,
    pub exclude: Option<Regex>,
}

#[derive(Clone)]
pub struct RequestInsertStatic {
    pub name: HeaderName,
    pub value: HeaderValue,
}

#[derive(Clone)]
pub struct ResponseInsertStatic {
    pub name: HeaderName,
    pub value: HeaderValue,
}

#[derive(Clone)]
pub struct RequestRemoveNamed {
    pub names: Vec<HeaderName>,
}

#[derive(Clone)]
pub struct ResponseRemoveNamed {
    pub names: Vec<HeaderName>,
}

#[derive(Clone)]
pub struct RequestRemoveRegex {
    pub regex: Regex,
}

#[derive(Clone)]
pub struct ResponseRemoveRegex {
    pub regex: Regex,
}

#[derive(Clone)]
pub enum ResponseHeaderRule {
    PropagateNamed(ResponsePropagateNamed),
    PropagateRegex(ResponsePropagateRegex),
    InsertStatic(ResponseInsertStatic),
    RemoveNamed(ResponseRemoveNamed),
    RemoveRegex(ResponseRemoveRegex),
}

#[derive(Clone)]
pub struct ResponsePropagateNamed {
    pub names: Vec<HeaderName>,
    pub rename: Option<HeaderName>,
    pub default: Option<HeaderValue>,
    pub strategy: HeaderAggregationStrategy,
}

#[derive(Clone)]
pub struct ResponsePropagateRegex {
    pub include: Option<Regex>,
    pub exclude: Option<Regex>,
    pub strategy: HeaderAggregationStrategy,
}

#[derive(Clone, Copy)]
pub enum HeaderAggregationStrategy {
    First,
    Last,
    Append,
}

type AggregatedHeader = (HeaderAggregationStrategy, Vec<HeaderValue>);

#[derive(Default)]
pub struct ResponseHeaderAggregator {
    pub entries: HashMap<HeaderName, AggregatedHeader>,
}
