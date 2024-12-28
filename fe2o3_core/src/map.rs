//! Generic map traits and implementations.
//! 
//! This module provides trait abstractions over different map types like `HashMap` and `BTreeMap`,
//! allowing for backend-agnostic map usage. It includes basic map operations, error handling, and
//! recursive map lookups.
//!
//! # Note on `Borrow` Support
//! Currently, the `get` method on the `Map` trait cannot support the full flexibility of
//! `std::borrow::Borrow` (e.g., using `&str` to look up `String` keys). This is because
//! `HashMap::get` requires the borrowed type to implement `Hash`, while `BTreeMap::get` requires
//! `Ord`. Without specialization (currently unstable in Rust), we cannot have different trait bounds
//! for different implementations of the same trait method.
//!
//! This limitation will be addressed once specialization becomes available in stable Rust, allowing
//! for more flexible key lookups while maintaining the efficiency of the underlying map implementations.
//! 
//! # Examples
//! ```
//! use std::collections::HashMap;
//! use crate::map::Map;
//! 
//! let mut map = HashMap::new();
//! map.insert("key".to_string(), 42);
//! 
//! // Currently must use String for lookup, &str not supported yet
//! assert_eq!(map.get(&"key".to_string()), Some(&42));
//! ```
use crate::{
    prelude::*,
};

use std::{
    //borrow::Borrow,
    collections::{
        btree_map,
        hash_map,
        BTreeMap,
        HashMap,
    },
    fmt::Display,
    hash::Hash,
    marker::PhantomData,
};


pub trait GetOrErr<K, V> {
    fn get_or_err(&self, k: &K) -> Outcome<&V>;
}

impl< K: Display + Hash + Eq, V> GetOrErr<K, V> for HashMap<K, V> {

    #[inline(always)]
    fn get_or_err(&self, k: &K) -> Outcome<&V> {
        match self.get(k) {
            Some(v) => Ok(v),
            None => Err(err!(
                "Map missing {}", k;
            Missing, Key)),
        }
    }
}

impl< K: Display + Ord, V> GetOrErr<K, V> for BTreeMap<K, V> {

    #[inline(always)]
    fn get_or_err(&self, k: &K) -> Outcome<&V> {
        match self.get(k) {
            Some(v) => Ok(v),
            None => Err(err!(
                "Map missing {}", k;
            Missing, Key)),
        }
    }
}

//======= Generic Maps ========================================================
//
// Inspired by https://github.com/bbqsrc/collections and
// https://jackh726.github.io/rust/2022/05/04/a-shiny-future-with-gats.html
//
pub trait Map<K, V> {

    type Iter<'iter>: Iterator<Item = (&'iter K, &'iter V)>
        where Self: 'iter, K: 'iter, V: 'iter;

    fn empty() -> Self;
    fn len(&self) -> usize;
    fn get(&self, k: &K) -> Option<&V>;
    fn contains_key(&self, k: &K) -> bool;
    fn iter<'a>(&'a self) -> Self::Iter<'a>;
}

pub trait MapMut<K, V>: Map<K, V> {

    type IterMut<'iter>: Iterator<Item = (&'iter K, &'iter mut V)>
        where Self: 'iter, K: 'iter, V: 'iter;

    fn get_mut(&mut self, k: &K) -> Option<&mut V>;
    fn insert(&mut self, k: K, v: V) -> Option<V>;
    fn remove(&mut self, k: &K) -> Option<V>;
    fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, f: F);
    fn iter_mut<'a>(&'a mut self) -> Self::IterMut<'a>;
}

impl<K: Hash + Eq, V> Map<K, V> for HashMap<K, V> {

    type Iter<'iter> = hash_map::Iter<'iter, K, V> where Self: 'iter;

    #[inline(always)]
    fn empty() -> Self { HashMap::new() }
    #[inline(always)]
    fn len(&self) -> usize { self.len() }
    #[inline(always)]
    fn get(&self, k: &K) -> Option<&V> { self.get(k) }
    #[inline(always)]
    fn contains_key(&self, k: &K) -> bool { self.contains_key(k) }
    #[inline(always)]
    fn iter<'a>(&'a self) -> Self::Iter<'a> { self.iter() }
}

impl<K: Hash + Eq, V> MapMut<K, V> for HashMap<K, V> {

    type IterMut <'iter> = hash_map::IterMut<'iter, K, V> where Self: 'iter;

    #[inline(always)]
    fn get_mut(&mut self, k: &K) -> Option<&mut V> { self.get_mut(k) }
    #[inline(always)]
    fn insert(&mut self, k: K, v: V) -> Option<V> { self.insert(k, v) }
    #[inline(always)]
    fn remove(&mut self, k: &K) -> Option<V> { self.remove(k) }
    #[inline(always)]
    fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, f: F) { self.retain(f); }
    #[inline(always)]
    fn iter_mut<'a>(&'a mut self) -> Self::IterMut<'a> { self.iter_mut() }
}

impl<K: Ord, V> Map<K, V> for BTreeMap<K, V> {

    type Iter<'iter> = btree_map::Iter<'iter, K, V> where Self: 'iter;

    #[inline(always)]
    fn empty() -> Self { BTreeMap::new() }
    #[inline(always)]
    fn len(&self) -> usize { self.len() }
    #[inline(always)]
    fn get(&self, k: &K) -> Option<&V> { self.get(k) }
    #[inline(always)]
    fn contains_key(&self, k: &K) -> bool { self.contains_key(k) }
    #[inline(always)]
    fn iter<'a>(&'a self) -> Self::Iter<'a> { self.iter() }
}

impl<K: Ord, V> MapMut<K, V> for BTreeMap<K, V> {

    type IterMut <'iter> = btree_map::IterMut<'iter, K, V> where Self: 'iter;

    #[inline(always)]
    fn get_mut(&mut self, k: &K) -> Option<&mut V> { self.get_mut(k) }
    #[inline(always)]
    fn insert(&mut self, k: K, v: V) -> Option<V> { self.insert(k, v) }
    #[inline(always)]
    fn remove(&mut self, k: &K) -> Option<V> { self.remove(k) }
    #[inline(always)]
    fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, f: F) { self.retain(f); }
    #[inline(always)]
    fn iter_mut<'a>(&'a mut self) -> Self::IterMut<'a> { self.iter_mut() }
}

impl<K: 'static, V: 'static> Map<K, V> for () {
    type Iter<'a> = EmptyIter<'a, K, V>;

    #[inline(always)]
    fn empty() -> Self { () }
    #[inline(always)]
    fn len(&self) -> usize { 0 }
    #[inline(always)]
    fn get(&self, _k: &K) -> Option<&V> { None }
    #[inline(always)]
    fn contains_key(&self, _k: &K) -> bool { false }
    #[inline(always)]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        EmptyIter(
            PhantomData::<&()>,
            PhantomData::<K>,
            PhantomData::<V>,
        )
    }
}

impl<K: 'static, V: 'static> MapMut<K, V> for () {
    type IterMut<'a> = EmptyIterMut<'a, K, V>;

    #[inline(always)]
    fn get_mut(&mut self, _k: &K) -> Option<&mut V> { None }
    #[inline(always)]
    fn insert(&mut self, _k: K, _v: V) -> Option<V> { None }
    #[inline(always)]
    fn remove(&mut self, _k: &K) -> Option<V> { None }
    #[inline(always)]
    fn retain<F>(&mut self, _f: F) where F: FnMut(&K, &mut V) -> bool, { }
    #[inline(always)]
    fn iter_mut<'a>(&'a mut self) -> Self::IterMut<'a> {
        EmptyIterMut(
            PhantomData::<&mut ()>,
            PhantomData::<K>,
            PhantomData::<V>,
        )
    }
}

pub struct EmptyIter<'a, K, V>(
    PhantomData<&'a ()>,
    PhantomData<K>,
    PhantomData<V>,
);

impl<'a, K: 'a, V: 'a> Iterator for EmptyIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> { None }
}

pub struct EmptyIterMut<'a, K, V>(
    PhantomData<&'a mut ()>,
    PhantomData<K>,
    PhantomData<V>,
);

impl<'a, K: 'a, V: 'a> Iterator for EmptyIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> { None }
}

//======= Recursive Maps ======================================================
// Basically works by wrapping every value in an enum, delineating it as a value or another key for
// recursive dereferncing.
//

pub trait KeyOrVal<K, V> {
    fn key(&self) -> Option<&K>;
    fn val(&self) -> Option<&V>;
}

pub trait MapRec<'a, K, V, KV>: Map<K, KV> where KV: KeyOrVal<K, V> + 'a {
    fn get_recursive(&'a self, k: &K) -> Option<&'a V> {
        match self.get(k) {
            Some(kv) => match kv.key() {
                Some(k2) => self.get_recursive(k2),
                None => kv.val(),
            },
            None => None,
        }
    }
}

impl<'a, K: Hash + Eq, V, KV> MapRec<'a, K, V, KV> for HashMap<K, KV> where KV: KeyOrVal<K, V> + 'a {}
impl<'a, K: Ord, V, KV> MapRec<'a, K, V, KV> for BTreeMap<K, KV> where KV: KeyOrVal<K, V> + 'a {}

#[derive(Clone, Debug)]
pub enum Recursive<K, V> {
    Key(K),
    Val(V),
}

impl<K, V> KeyOrVal<K, V> for Recursive<K, V> {
    fn key(&self) -> Option<&K> {
        match self {
            Self::Key(k) => Some(k),
            Self::Val(_) => None,
        }
    }
    fn val(&self) -> Option<&V> {
        match self {
            Self::Key(_) => None,
            Self::Val(v) => Some(v),
        }
    }
}


//// Old... 
//pub trait HashMapKey: Debug + Eq + Hash {}
//pub trait BTreeMapKey: Debug + Eq + Ord {}
//
//#[derive(Clone, Debug, Hash)]
//pub enum HashMapEntry<K, V> 
//where
//    K: HashMapKey,
//{
//    Key(K),
//    Value(V),
//}
//
//pub trait RecursiveHashMapEntry<K, V>
//where
//    K: HashMapKey,
//{
//    fn as_entry(&self) -> &HashMapEntry<K, V>;
//}
//
//impl<K, V> RecursiveHashMapEntry<K, V> for HashMapEntry<K, V>
//where
//    K: HashMapKey,
//{
//    fn as_entry(&self) -> &Self {
//        self
//    }
//}
//
//pub trait RecursiveHashMap<K, V> 
//where
//    K: HashMapKey,
//{
//    fn get_recursive<Q: ?Sized>(&self, k: &Q) -> Option<&V>
//    where
//        K: Borrow<Q>,
//        Q: HashMapKey;
//}
//
//impl<K, V1, V2> RecursiveHashMap<K, V2> for HashMap<K, V1> 
//where
//    K: HashMapKey,
//    V1: RecursiveHashMapEntry<K, V2>,
//{
//    fn get_recursive<Q: ?Sized>(&self, k: &Q) -> Option<&V2>
//    where
//        K: Borrow<Q>,
//        Q: HashMapKey,
//    {
//        match self.get(k) {
//            Some(e) => match e.as_entry() {
//                HashMapEntry::Key(k) => self.get_recursive(k.borrow()),
//                HashMapEntry::Value(v) => Some(v),
//            },
//            None => None,
//        }
//    }
//}
//
//#[derive(Clone, Debug)]
//pub enum BTreeMapEntry<K, V> 
//where
//    K: BTreeMapKey,
//{
//    Key(K),
//    Value(V),
//}
//
//pub trait RecursiveBTreeMapEntry<K, V>
//where
//    K: BTreeMapKey,
//{
//    fn as_entry(&self) -> &BTreeMapEntry<K, V>;
//}
//
//impl<K, V> RecursiveBTreeMapEntry<K, V> for BTreeMapEntry<K, V>
//where
//    K: BTreeMapKey,
//{
//    fn as_entry(&self) -> &Self {
//        self
//    }
//}
//
//pub trait RecursiveBTreeMap<K, V> 
//where
//    K: BTreeMapKey,
//{
//    fn get_recursive<Q: ?Sized>(&self, k: &Q) -> Option<&V>
//    where
//        K: Borrow<Q>,
//        Q: BTreeMapKey;
//}
//
//impl<K, V1, V2> RecursiveBTreeMap<K, V2> for BTreeMap<K, V1> 
//where
//    K: BTreeMapKey,
//    V1: RecursiveBTreeMapEntry<K, V2>,
//{
//    fn get_recursive<Q: ?Sized>(&self, k: &Q) -> Option<&V2>
//    where
//        K: Borrow<Q>,
//        Q: BTreeMapKey,
//    {
//        match self.get(k) {
//            Some(e) => match e.as_entry() {
//                BTreeMapEntry::Key(k) => self.get_recursive(k.borrow()),
//                BTreeMapEntry::Value(v) => Some(v),
//            },
//            None => None,
//        }
//    }
//}
//
//impl HashMapKey for String {}
//impl HashMapKey for str {}
//impl HashMapKey for &'static str {}
//impl HashMapKey for u8 {}
//impl HashMapKey for u16 {}
//impl HashMapKey for u32 {}
//impl HashMapKey for u64 {}
//
//impl BTreeMapKey for String {}
//impl BTreeMapKey for str {}
//impl BTreeMapKey for &'static str {}
//impl BTreeMapKey for u8 {}
//impl BTreeMapKey for u16 {}
//impl BTreeMapKey for u32 {}
//impl BTreeMapKey for u64 {}
////impl BTreeMapKey for [u8; 32] {}

//#[cfg(test)]
//mod tests {
//    use super::*;
//    use crate::prelude::*;
//    
//    //#[test]
//    //fn test_recursive_hashmap_simple_get() -> Outcome<()> {
//    //    let mut map: HashMap<u8, HashMapEntry<u8, u8>> = HashMap::new();
//    //    map.insert(1, HashMapEntry::Value(1));
//    //    map.insert(2, HashMapEntry::Value(2));
//    //    map.insert(3, HashMapEntry::Key(1));
//    //    assert_eq!(map.get_recursive(&3), Some(&1));
//    //    Ok(())
//    //}
//
//    //#[test]
//    //fn test_recursive_btreemap_simple_get() -> Outcome<()> {
//    //    let mut map: BTreeMap<u8, BTreeMapEntry<u8, u8>> = BTreeMap::new();
//    //    map.insert(1, BTreeMapEntry::Value(1));
//    //    map.insert(2, BTreeMapEntry::Value(2));
//    //    map.insert(3, BTreeMapEntry::Key(1));
//    //    assert_eq!(map.get_recursive(&3), Some(&1));
//    //    Ok(())
//    //}
//
//    #[test]
//    fn test_generic_recursive_map_00() -> Outcome<()> {
//        let mut map: BTreeMap<u8, Recursive<u8, u8>> = BTreeMap::new();
//        map.insert(1, Recursive::Val(1));
//        map.insert(2, Recursive::Val(2));
//        map.insert(3, Recursive::Key(1));
//        assert_eq!(map.get_recursive(&3), Some(&1));
//        Ok(())
//    }
//
//    #[test]
//    fn test_generic_recursive_map_01() -> Outcome<()> {
//        let mut map: HashMap<u8, Recursive<u8, u8>> = HashMap::new();
//        map.insert(1, Recursive::Val(1));
//        map.insert(2, Recursive::Val(2));
//        map.insert(3, Recursive::Key(1));
//        assert_eq!(map.get_recursive(&3), Some(&1));
//        Ok(())
//    }
//}
