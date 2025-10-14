use hive_router_query_planner::consumer_schema::ConsumerSchema;

pub struct OnSchemaReloadPayload {
    pub old_schema: &'static ConsumerSchema,
    pub new_schema: &'static mut ConsumerSchema,
}
