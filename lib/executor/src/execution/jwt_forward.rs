use sonic_rs::Value;

#[derive(Default)]
pub struct JwtAuthForwardingPlan {
    pub extension_field_name: String,
    pub extension_field_value: Value,
}
