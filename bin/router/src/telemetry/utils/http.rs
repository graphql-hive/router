use http::{uri::Scheme, Method, Uri, Version};

pub trait HttpUriAsStr {
    fn scheme_static_str(&self) -> &'static str;
}

impl HttpUriAsStr for Uri {
    fn scheme_static_str(&self) -> &'static str {
        if self.scheme() == Some(&Scheme::HTTPS) {
            "https"
        } else {
            "http"
        }
    }
}

pub trait HttpVersionAsStr {
    fn as_static_str(&self) -> &'static str;
}

impl HttpVersionAsStr for Version {
    fn as_static_str(&self) -> &'static str {
        match *self {
            Version::HTTP_09 => "0.9",
            Version::HTTP_10 => "1.0",
            Version::HTTP_11 => "1.1",
            Version::HTTP_2 => "2",
            Version::HTTP_3 => "3",
            // SAFETY: only supported HTTP versions will ever reach router
            _ => unreachable!("Unknown HTTP version"),
        }
    }
}

pub trait HttpMethodAsStr {
    fn as_static_str(&self) -> &'static str;
}

impl HttpMethodAsStr for Method {
    fn as_static_str(&self) -> &'static str {
        match *self {
            Method::GET => "GET",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::PATCH => "PATCH",
            Method::DELETE => "DELETE",
            Method::HEAD => "HEAD",
            Method::OPTIONS => "OPTIONS",
            Method::CONNECT => "CONNECT",
            Method::TRACE => "TRACE",
            _ => {
                if self.as_str() == "QUERY" {
                    // Special case for QUERY method,
                    // that is not yet stable
                    "QUERY"
                } else {
                    // For telemetry purposes, we log the method as "_OTHER"
                    "_OTHER"
                }
            }
        }
    }
}

pub fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}
