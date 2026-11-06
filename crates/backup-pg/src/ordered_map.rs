use std::{collections::HashMap, fmt::Debug, hash::Hash, ops::Deref};

use serde::{Serialize, Serializer, ser::SerializeMap};

#[derive(Clone, Debug)]
pub struct OMap<K, V> {
    map: HashMap<K, V>,
    order: Vec<K>,
}

impl<K, V> OMap<K, V>
where
    K: Eq + PartialEq + Hash + Clone,
{
    pub fn new() -> Self {
        OMap {
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        OMap {
            map: HashMap::with_capacity(capacity),
            order: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn insert(&mut self, key: K, value: V) {
        if self.order.contains(&key) {
            let index = self.order.iter().position(|x| *x == key).unwrap();
            self.order.remove(index);
        }

        self.order.push(key.clone());
        self.map.insert(key, value);
    }

    #[inline]
    pub fn to_serialize<'a>(&'a self) -> Vec<(&'a K, &'a V)> {
        self.order
            .iter()
            .map(|v| (v, self.map.get(v).unwrap()))
            .collect::<Vec<(&'a K, &'a V)>>()
    }
}

impl<K, V> Deref for OMap<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<K, V> Serialize for OMap<K, V>
where
    K: Serialize + Eq + PartialEq + Hash + Clone,
    V: Serialize,
{
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self.to_serialize() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}
