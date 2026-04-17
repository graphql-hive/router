use hive_router_internal::expressions::lib::{ToVrlValue, VrlObjectBuilder};
use hive_router_internal::expressions::VrlView;
use ntex::http::{HeaderMap, Method, Uri};
use ntex::web::{self, DefaultError};

use crate::coprocessor::runtime::MutableRequestState;
use crate::plugins::hooks::on_graphql_params::GraphQLParams;

pub trait RequestLike {
    fn method(&self) -> &Method;
    fn headers(&self) -> &HeaderMap;
    fn uri(&self) -> &Uri;
}

pub trait ResponseLike {
    type Request: RequestLike + ?Sized;

    fn headers(&self) -> &HeaderMap;
    fn status_code(&self) -> u16;
    fn request(&self) -> &Self::Request;
}

pub trait OperationLike {
    fn operation_name(&self) -> Option<&str>;
    fn query(&self) -> Option<&str>;
}

pub struct RequestView<'a, T: ?Sized> {
    pub source: &'a T,
}

impl<T> VrlView for RequestView<'_, T>
where
    T: RequestLike + ?Sized,
{
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        out.insert_lazy("method", || self.source.method().as_str().into())
            .insert_lazy("headers", || self.source.headers().to_vrl_value())
            .insert_lazy("url", || self.source.uri().to_vrl_value());
    }
}

pub struct ResponseView<'a, T: ?Sized> {
    pub source: &'a T,
}

impl<T> VrlView for ResponseView<'_, T>
where
    T: ResponseLike + ?Sized,
{
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        out.insert_lazy("headers", || self.source.headers().to_vrl_value())
            .insert_lazy("status_code", || self.source.status_code().into());
    }
}

pub struct OperationView<'a, T: ?Sized> {
    pub source: &'a T,
}

impl<T> VrlView for OperationView<'_, T>
where
    T: OperationLike + ?Sized,
{
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        out.insert_object("operation", |op| {
            op.insert_lazy("name", || {
                self.source.operation_name().map(str::to_owned).into()
            })
            .insert_lazy("query", || self.source.query().map(str::to_owned).into());
        });
    }
}

pub struct RequestContextView<'a, Req: ?Sized, Op: ?Sized = ()> {
    request: &'a Req,
    operation: Option<&'a Op>,
}

impl<'a, Req> RequestContextView<'a, Req>
where
    Req: RequestLike + ?Sized,
{
    pub fn new(request: &'a Req) -> Self {
        Self {
            request,
            operation: None,
        }
    }

    pub fn with_operation<Op>(self, operation: &'a Op) -> RequestContextView<'a, Req, Op>
    where
        Op: OperationLike + ?Sized,
    {
        RequestContextView {
            request: self.request,
            operation: Some(operation),
        }
    }
}

impl<Req, Op> VrlView for RequestContextView<'_, Req, Op>
where
    Req: RequestLike + ?Sized,
    Op: OperationLike + ?Sized,
{
    fn write<'a>(&self, root: &mut VrlObjectBuilder<'a, '_>) {
        root.insert_object("request", |req| {
            RequestView {
                source: self.request,
            }
            .write(req);

            if let Some(operation) = self.operation {
                OperationView { source: operation }.write(req);
            }
        });
    }
}

pub struct RequestResponseContextView<'a, Res: ?Sized> {
    pub response: &'a Res,
}

impl<Res> VrlView for RequestResponseContextView<'_, Res>
where
    Res: ResponseLike + ?Sized,
{
    fn write<'a>(&self, root: &mut VrlObjectBuilder<'a, '_>) {
        root.insert_object("request", |req| {
            RequestView {
                source: self.response.request(),
            }
            .write(req);
        })
        .insert_object("response", |res| {
            ResponseView {
                source: self.response,
            }
            .write(res);
        });
    }
}

impl RequestLike for web::HttpRequest {
    fn method(&self) -> &Method {
        self.method()
    }

    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn uri(&self) -> &Uri {
        self.uri()
    }
}

impl RequestLike for web::WebRequest<DefaultError> {
    fn method(&self) -> &Method {
        self.method()
    }

    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn uri(&self) -> &Uri {
        self.uri()
    }
}

impl RequestLike for MutableRequestState<'_> {
    fn method(&self) -> &Method {
        self.method
    }

    fn headers(&self) -> &HeaderMap {
        self.headers
    }

    fn uri(&self) -> &Uri {
        self.uri
    }
}

impl ResponseLike for web::WebResponse {
    type Request = web::HttpRequest;

    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn status_code(&self) -> u16 {
        self.response().status().as_u16()
    }

    fn request(&self) -> &Self::Request {
        self.request()
    }
}

impl OperationLike for GraphQLParams {
    fn operation_name(&self) -> Option<&str> {
        self.operation_name.as_deref()
    }

    fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
}

impl OperationLike for () {
    fn operation_name(&self) -> Option<&str> {
        None
    }

    fn query(&self) -> Option<&str> {
        None
    }
}
