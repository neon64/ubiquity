use fnv::FnvHasher;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::hash::{Hash, Hasher};

pub fn hash_value<T: Hash + ?Sized>(object: &T) -> u64 {
    let mut hasher: FnvHasher = Default::default();
    object.hash(&mut hasher);
    hasher.finish()
}

pub type FnvHashMap<K, T> = HashMap<K, T, BuildHasherDefault<FnvHasher>>;
