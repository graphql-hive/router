use std::{collections::HashMap, str::FromStr};
use tonic::metadata::{MetadataKey, MetadataMap};

pub(super) fn build_metadata(headers: HashMap<String, String>) -> MetadataMap {
    let metadata = tonic::metadata::MetadataMap::with_capacity(headers.len());

    headers
        .into_iter()
        .fold(metadata, |mut acc, (header_name, header_value)| {
            let key = MetadataKey::from_str(header_name.as_str()).unwrap();
            acc.insert(key, header_value.as_str().parse().unwrap());
            acc
        })
}
