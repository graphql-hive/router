use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct SubgraphOperationNameConfig {
    default_forward: bool,
    subgraphs: BTreeMap<String, bool>,
}

impl SubgraphOperationNameConfig {
    pub fn new(default_forward: bool, subgraphs: BTreeMap<String, bool>) -> Self {
        Self {
            default_forward,
            subgraphs,
        }
    }

    pub fn should_forward(&self, subgraph_name: &str) -> bool {
        self.subgraphs
            .get(subgraph_name)
            .copied()
            .unwrap_or(self.default_forward)
    }

    pub fn operation_name(
        &self,
        subgraph_name: &str,
        client_operation_name: Option<&str>,
        fetch_step_id: i64,
    ) -> Option<String> {
        if self.should_forward(subgraph_name) {
            client_operation_name
                .filter(|name| !name.is_empty())
                .map(|name| format!("{}_{}", name, fetch_step_id))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_name_is_not_generated_for_empty_client_operation_name() {
        let config = SubgraphOperationNameConfig::new(true, BTreeMap::new());

        assert_eq!(config.operation_name("products", Some(""), 7), None);
    }
}
