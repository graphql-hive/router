use crate::jwt::context::JwtRequestContext;

pub struct RequestContext {
    pub jwt: Option<JwtRequestContext>,
}

impl RequestContext {
    pub fn new() -> Self {
        RequestContext { jwt: None }
    }
}
