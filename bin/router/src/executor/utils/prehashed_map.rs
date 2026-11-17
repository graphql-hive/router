use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
};

/// A map keyed by hashes we already have, like the hash of an entity. The key goes to the map
/// as its own hash, so it isn't hashed twice.
pub type PrehashedMap<T> = HashMap<u64, T, BuildHasherDefault<Prehashed>>;

#[derive(Default)]
pub struct Prehashed(u64);

impl Hasher for Prehashed {
    fn write(&mut self, _: &[u8]) {
        unreachable!("only u64 keys")
    }

    fn write_u64(&mut self, hash: u64) {
        self.0 = hash;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
