use std::{collections::HashSet, sync::MutexGuard};

use hive_router_query_planner::state::supergraph_state::OperationKind;

use crate::request_context::{RequestContext, RequestContextError, SharedRequestContext};

impl SharedRequestContext {
    pub fn for_plugin(&self) -> RequestContextPluginApi {
        RequestContextPluginApi::new(self.clone())
    }
}

#[derive(Clone)]
pub struct RequestContextPluginApi {
    context: SharedRequestContext,
}

pub struct RequestContextPluginRead {
    snapshot: RequestContext,
}

pub struct RequestContextPluginRef<'a> {
    pub operation_name: Option<&'a String>,
    pub operation_kind: Option<&'a OperationKind>,
    pub unresolved_labels: Option<&'a HashSet<String>>,
    pub labels_to_override: Option<&'a HashSet<String>>,
}

pub struct RequestContextPluginWrite<'a> {
    context: MutexGuard<'a, RequestContext>,
}

impl RequestContextPluginApi {
    fn new(context: SharedRequestContext) -> RequestContextPluginApi {
        RequestContextPluginApi { context }
    }

    pub fn read(&self) -> Result<RequestContextPluginRead, RequestContextError> {
        Ok(RequestContextPluginRead {
            snapshot: self.context.snapshot()?,
        })
    }

    pub fn write(&self) -> Result<RequestContextPluginWrite<'_>, RequestContextError> {
        Ok(RequestContextPluginWrite {
            context: self.context.lock()?,
        })
    }
}

impl RequestContextPluginRead {
    pub fn as_ref(&self) -> RequestContextPluginRef<'_> {
        let operation = &self.snapshot.operation;
        let progressive_override = &self.snapshot.progressive_override;

        RequestContextPluginRef {
            operation_name: operation.name.as_ref(),
            operation_kind: operation.kind.as_ref(),
            unresolved_labels: progressive_override.unresolved_labels.as_ref(),
            labels_to_override: progressive_override.labels_to_override.as_ref(),
        }
    }
}

impl RequestContextPluginWrite<'_> {
    pub fn set_labels_to_override(&mut self, labels: Option<HashSet<String>>) -> &mut Self {
        self.context.progressive_override.labels_to_override = labels;
        self
    }

    pub fn clear_labels_to_override(&mut self) -> &mut Self {
        self.context.progressive_override.labels_to_override = None;
        self
    }
}
