use hive_router_query_planner::ast::selection_item::SelectionItem;
use sonic_rs::json;

use crate::{
    execution::plan::PlanExecutionOutput,
    hooks::on_query_plan::{OnQueryPlanEndPayload, OnQueryPlanStartPayload},
    plugin_trait::{HookResult, RouterPlugin, StartPayload},
};

pub struct RootFieldLimitPlugin {
    pub max_root_fields: usize,
}

#[async_trait::async_trait]
impl RouterPlugin for RootFieldLimitPlugin {
    async fn on_query_plan<'exec>(
        &'exec self,
        payload: OnQueryPlanStartPayload<'exec>,
    ) -> HookResult<'exec, OnQueryPlanStartPayload<'exec>, OnQueryPlanEndPayload> {
        let mut cnt = 0;
        for selection in payload
            .filtered_operation_for_plan
            .selection_set
            .items
            .iter()
        {
            match selection {
                SelectionItem::Field(_) => {
                    cnt += 1;
                    if cnt > self.max_root_fields {
                        let err_msg = format!(
                            "Query has too many root fields: {}, maximum allowed is {}",
                            cnt, self.max_root_fields
                        );
                        tracing::warn!("{}", err_msg);
                        let body = json!({
                            "errors": [{
                                "message": err_msg,
                                "extensions": {
                                    "code": "TOO_MANY_ROOT_FIELDS"
                                }
                            }]
                        });
                        // Return error
                        return payload.end_response(PlanExecutionOutput {
                            body: sonic_rs::to_vec(&body).unwrap_or_default(),
                            headers: http::HeaderMap::new(),
                            status: http::StatusCode::PAYLOAD_TOO_LARGE,
                        });
                    }
                }
                SelectionItem::InlineFragment(_) => {
                    unreachable!("Inline fragments should have been inlined before query planning");
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("Fragment spreads should have been inlined before query planning");
                }
            }
        }
        payload.cont()
    }
}
