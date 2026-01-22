pub enum PathSegment<'a> {
    Field(&'a str),
    Index(usize),
}

/// Represents an observed error with all necessary metadata extracted.
/// This struct should be created lazily, only when the span is enabled.
pub struct ObservedError {
    pub code: Option<String>,
    pub message: String,
    pub path: Option<String>,
    pub service_name: Option<String>,
    pub affected_path: Option<String>,
}

impl ObservedError {
    /// Helper function to format a path iterator into a string
    pub fn format_path<'a>(path: impl Iterator<Item = PathSegment<'a>>) -> String {
        let mut path_str = String::new();
        let mut first = true;
        for segment in path {
            if !first {
                path_str.push('.');
            }
            match segment {
                PathSegment::Field(name) => path_str.push_str(name),
                PathSegment::Index(idx) => {
                    use std::fmt::Write;
                    write!(path_str, "{}", idx).unwrap();
                }
            }
            first = false;
        }
        path_str
    }
}
