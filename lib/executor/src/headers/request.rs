use http::HeaderMap;
use ntex_http::HeaderMap as NtexHeaderMap;

use crate::headers::{
    plan::{
        HeaderRulesPlan, RequestHeaderRule, RequestInsertStatic, RequestPropagateNamed,
        RequestPropagateRegex, RequestRemoveNamed, RequestRemoveRegex,
    },
    sanitizer::{is_denied_header, is_never_join_header},
};

pub fn modify_subgraph_request_headers(
    header_rule_plan: &HeaderRulesPlan,
    subgraph_name: &str,
    client_headers: &NtexHeaderMap,
    output_headers: &mut HeaderMap,
) {
    let global_actions = &header_rule_plan.request.global;
    let subgraph_actions = header_rule_plan.request.by_subgraph.get(subgraph_name);

    for action in global_actions
        .iter()
        .chain(subgraph_actions.into_iter().flatten())
    {
        action.apply_request_headers(client_headers, output_headers);
    }
}

trait ApplyRequestHeader {
    fn apply_request_headers(&self, client_headers: &NtexHeaderMap, output_headers: &mut HeaderMap);
}

impl ApplyRequestHeader for RequestHeaderRule {
    fn apply_request_headers(
        &self,
        client_headers: &NtexHeaderMap,
        output_headers: &mut HeaderMap,
    ) {
        match self {
            Self::PropagateNamed(data) => {
                data.apply_request_headers(client_headers, output_headers)
            }
            Self::PropagateRegex(data) => {
                data.apply_request_headers(client_headers, output_headers)
            }
            Self::InsertStatic(data) => data.apply_request_headers(client_headers, output_headers),
            Self::RemoveNamed(data) => data.apply_request_headers(client_headers, output_headers),
            Self::RemoveRegex(data) => data.apply_request_headers(client_headers, output_headers),
        }
    }
}

impl ApplyRequestHeader for RequestPropagateNamed {
    fn apply_request_headers(
        &self,
        client_headers: &NtexHeaderMap,
        output_headers: &mut HeaderMap,
    ) {
        let mut matched = false;

        for header_name in &self.names {
            if is_denied_header(header_name) {
                continue;
            }
            if let Some(header_value) = client_headers.get(header_name) {
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
        client_headers: &NtexHeaderMap,
        output_headers: &mut HeaderMap,
    ) {
        for (header_name, header_value) in client_headers {
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
        _client_headers: &NtexHeaderMap,
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

impl ApplyRequestHeader for RequestRemoveNamed {
    fn apply_request_headers(
        &self,
        _client_headers: &NtexHeaderMap,
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
        _client_headers: &NtexHeaderMap,
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
