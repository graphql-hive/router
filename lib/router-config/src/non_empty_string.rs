pub trait NoneIfEmpty {
    fn none_if_empty(self) -> Option<String>
    where
        Self: Sized;
}

impl NoneIfEmpty for Option<String> {
    fn none_if_empty(self) -> Option<String> {
        self.filter(|s| !s.is_empty())
    }
}
