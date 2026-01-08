pub enum PathSegment<'a> {
    Field(&'a str),
    Index(usize),
}

pub trait ObservedGraphQLError {
    fn get_code(&self) -> Option<&str>;
    fn get_message(&self) -> &str;
    fn get_path(&self) -> Option<impl Iterator<Item = PathSegment<'_>> + '_>;
    fn get_service_name(&self) -> Option<&str>;
    fn get_affected_path(&self) -> Option<&str>;
}
