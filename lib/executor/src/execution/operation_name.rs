use std::{collections::HashSet, sync::Arc};

use hive_router_config::traffic_shaping::TrafficShapingConfig;

#[derive(Debug, Clone, Default)]
pub enum OperationNameForwardConfig {
    #[default]
    None,
    All,
    Only(HashSet<String>),
}

impl OperationNameForwardConfig {
    pub fn new<'a, I>(config: &'a TrafficShapingConfig, known_subgraph_names: I) -> Self
    where
        I: IntoIterator<Item = &'a String>,
    {
        if config.all.forward_operation_name {
            // enabled for all, so collect disabled
            let disabled_subgraphs = config
                .subgraphs
                .iter()
                .filter_map(|(name, config)| {
                    matches!(config.forward_operation_name, Some(false)).then_some(name.as_str())
                })
                .collect::<HashSet<_>>();

            return Self::all_except(known_subgraph_names, &disabled_subgraphs);
        }

        // disabled for all, so collect enabled
        let enabled_subgraphs = config
            .subgraphs
            .iter()
            .filter_map(|(name, config)| {
                matches!(config.forward_operation_name, Some(true)).then_some(name.clone())
            })
            .collect::<HashSet<_>>();

        Self::only(enabled_subgraphs)
    }

    pub fn none() -> Self {
        Self::None
    }

    pub fn all() -> Self {
        Self::All
    }

    pub fn only(subgraphs: HashSet<String>) -> Self {
        if subgraphs.is_empty() {
            Self::None
        } else {
            Self::Only(subgraphs)
        }
    }

    pub fn all_except<'a, I>(all_subgraphs: I, excluded_subgraphs: &HashSet<&'a str>) -> Self
    where
        I: IntoIterator<Item = &'a String>,
    {
        if excluded_subgraphs.is_empty() {
            return Self::All;
        }

        Self::only(
            all_subgraphs
                .into_iter()
                .filter(|subgraph| !excluded_subgraphs.contains(subgraph.as_str()))
                .map(|subgraph| subgraph.to_string())
                .collect(),
        )
    }

    pub fn should_forward(&self, subgraph_name: &str) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Only(subgraphs) => subgraphs.contains(subgraph_name),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OperationNameFactory {
    config: Arc<OperationNameForwardConfig>,
    client_operation_name: Option<Arc<str>>,
}

impl OperationNameFactory {
    pub fn new(
        config: Arc<OperationNameForwardConfig>,
        client_operation_name: Option<&str>,
    ) -> Self {
        Self {
            config,
            client_operation_name: client_operation_name.map(Arc::<str>::from),
        }
    }

    pub fn generate(&self, subgraph_name: &str, fetch_step_id: i64) -> Option<String> {
        if !self.config.should_forward(subgraph_name) {
            return None;
        }

        let client_operation_name = self.client_operation_name.as_deref()?;
        let mut fetch_step_id_buf = itoa::Buffer::new();
        let fetch_step_id = fetch_step_id_buf.format(fetch_step_id);
        let separator = "__";
        let mut operation_name = String::with_capacity(
            separator.len() + client_operation_name.len() + fetch_step_id.len(),
        );

        // Operation name does not need to be sanitized, so we use it raw
        operation_name.push_str(client_operation_name);
        operation_name.push_str(separator);
        operation_name.push_str(fetch_step_id);

        Some(operation_name)
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationNameFactory, OperationNameForwardConfig};
    use std::sync::Arc;

    #[test]
    fn sanitizes_subgraph_names() {
        let name =
            OperationNameFactory::new(Arc::new(OperationNameForwardConfig::all()), Some("Example"));

        assert_eq!(name.generate("foo", 2).as_deref(), Some("Example__2"));
        assert_eq!(name.generate("foo-v2", 2).as_deref(), Some("Example__2"));
        assert_eq!(
            name.generate("foo service", 3).as_deref(),
            Some("Example__3")
        );
        assert_eq!(name.generate("123foo", 4).as_deref(), Some("Example__4"));
    }
}
