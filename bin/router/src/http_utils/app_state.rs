use std::ops::Deref;

use ntex::http::Payload;
use ntex::web::{self, error::InternalError, FromRequest, HttpRequest};

/// Extracts a clone of an application state value registered with
/// [`web::WebAppConfig::set_state`], so handlers can receive it as an argument.
///
/// A request served by an app without the value responds with
/// `500 Internal Server Error`.
#[derive(Clone)]
pub struct AppData<T>(T);

impl<T> AppData<T> {
    /// Returns a reference to the extracted value.
    pub fn get_ref(&self) -> &T {
        &self.0
    }
}

impl<T> Deref for AppData<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone + 'static, St: web::State> FromRequest<St> for AppData<T> {
    type Error = InternalError<&'static str>;

    async fn from_request(
        _: &St,
        req: &HttpRequest,
        _: &mut Payload,
    ) -> Result<Self, Self::Error> {
        // a missing value is a wiring bug in the app setup, not a client error
        req.app_state::<T>()
            .cloned()
            .map(AppData)
            .ok_or_else(|| web::error::ErrorInternalServerError("app state is not configured"))
    }
}
