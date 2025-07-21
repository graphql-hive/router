use serde::Deserializer;
use std::num::NonZero;

/// Final representation of the response data after request execution.
#[derive(Debug)]
pub(crate) struct ResponseData<'req> {
    pub(super) root: ResponseObjectId,
    pub(super) parts: DataParts<'req>,
}

/// The response data is composed of multiple parts, each with its own objects and lists.
/// This allows subgraph request to be processed independently. Each object/list is uniquely
/// identifier by its DataPartId and PartObjectId/PartListId.
#[derive(Default, Debug)]
pub(crate) struct DataParts<'req>(Vec<DataPart<'req>>);

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub(crate) struct DataPartId(u16);

#[derive(Debug)]
pub(crate) struct DataPart<'req> {
    pub id: DataPartId,
    objects: Vec<ResponseObject<'req>>,
    lists: Vec<Vec<ResponseValue<'req>>>,
    maps: Vec<Vec<(&'req str, ResponseValue<'req>)>>,
}

#[derive(Debug, Default)]
pub(crate) struct ResponseObject<'req> {
    // pub(super) definition_id: Option<ObjectDefinitionId>,
    pub(super) fields: Vec<ResponseObjectField<'req>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResponseObjectField<'req> {
    pub key: PositionedResponseKey,
    pub value: ResponseValue<'req>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PositionedResponseKey {
    pub response_key: ResponseKey,
}

/// A ResponseKey is guaranteed to exist inside ResponseKeys
/// and thus will use `get_unchecked` to be retrieved. This improves
/// performance by around 1% since we're doing a binary search for each
/// incoming field name during deserialization.
#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ResponseKey(u16);

/// We keep track of whether a value is nullable or not for error propagation across plans
/// We include directly inside the ResponseValue as it'll be at least have the size of u64 + 1
/// word. As the enum variants don't need the full word, we might as well re-use that extra space
/// for something.
///
/// For the same reason we don't use a boxed slice for `List` to make it easier to for error
/// propagation to change a list item to null. So it's a slice id (offset + length in u32) into a
/// specific ResponseDataPart.
#[derive(Default, Debug, Clone)]
pub(crate) enum ResponseValue<'a> {
    #[default]
    Null,
    Boolean {
        value: bool,
    },
    // Defined as i32
    // https://spec.graphql.org/October2021/#sec-Int
    Int {
        value: &'a i32,
    },
    Float {
        value: &'a f64,
    },
    String {
        value: &'a str,
    },
    List {
        id: ResponseListId,
    },
    Object {
        id: ResponseObjectId,
    },
    // For Any, anything serde_json::Value would support
    I64 {
        value: &'a i64,
    },
    U64 {
        value: &'a u64,
    },
    Map {
        id: ResponseMapId,
    },
}

#[derive(
    Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct StringId(NonZero<u32>);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct ResponseObjectId {
    pub part_id: DataPartId,
    pub object_id: PartObjectId,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct PartObjectId(u32);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct ResponseListId {
    pub part_id: DataPartId,
    pub list_id: PartListId,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct PartListId(u32);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct PartMapId(u32);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct ResponseMapId {
    pub part_id: DataPartId,
    pub map_id: PartMapId,
}

// impl<'de> Deserializer<'de> for DataPart<'de> {
//   type Error = String;

//   fn deserialize_bool() -> Result
// }
