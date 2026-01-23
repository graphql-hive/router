pub trait NoneIfEmpty {
    fn none_if_empty(self) -> Option<String>
    where
        Self: Sized;
}

impl NoneIfEmpty for Option<String> {
    fn none_if_empty(self) -> Option<String> {
        match self {
            Some(s) if s.is_empty() => None,
            other => other,
        }
    }
}