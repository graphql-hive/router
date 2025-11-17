use hive_router_query_planner::consumer_schema::ConsumerSchema;

pub struct OnSchemaReloadPayload<'a> {
    pub old_schema: &'a ConsumerSchema,
    pub new_schema: &'a mut ConsumerSchema,
}
