pub struct JwtRequestContext {
    pub token: String,
}

impl JwtRequestContext {
    pub fn new(token: String) -> Self {
        JwtRequestContext { token }
    }
}
