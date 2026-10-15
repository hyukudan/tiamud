//! Copy-on-write containers for the authoritative world state.
//!
//! [`Shared`] makes [`World::checkpoint`](crate::World::checkpoint) a set of
//! reference-count bumps: a map is copied only when it is first mutated while
//! a checkpoint (or the durable baseline) still holds the previous version.
//! Every mutable access goes through `DerefMut`, which calls
//! [`Arc::make_mut`]. That single choke point is also what durable dirty
//! tracking relies on: while the durable baseline holds a reference, any
//! mutable access produces a new allocation, so pointer identity with the
//! baseline proves that no mutation path touched the value. A mutable access
//! that leaves the value unchanged is a conservative false positive.
//!
//! [`ItemInstances`] additionally owns a lazily built [`ItemLocationIndex`].
//! Any mutable access discards the index, so it can never be stale; the next
//! read rebuilds it from the map, and checkpoints share it.

use super::*;
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock},
};

/// Reference-counted copy-on-write wrapper. Semantics are those of `T`.
#[derive(Default)]
pub(crate) struct Shared<T>(Arc<T>);

impl<T> Shared<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// True when both handles point at the same allocation, which implies
    /// equal contents without comparing them.
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || *self.0 == *other.0
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a, T> IntoIterator for &'a Shared<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        (&*self.0).into_iter()
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut Shared<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0).into_iter()
    }
}

pub(crate) type ItemInstanceMap = BTreeMap<ItemInstanceId, ItemInstance>;

/// Secondary index from an item location to the instances stored there.
/// `BTreeSet` keeps iteration in item-ID order, which is the order the
/// former full scans over the `BTreeMap` produced.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct ItemLocationIndex {
    by_location: BTreeMap<SharedItemLocation, BTreeSet<ItemInstanceId>>,
}

impl ItemLocationIndex {
    pub(crate) fn build(instances: &ItemInstanceMap) -> Self {
        let mut by_location = BTreeMap::<SharedItemLocation, BTreeSet<ItemInstanceId>>::new();
        for (item_id, instance) in instances {
            by_location
                .entry(instance.location.clone())
                .or_default()
                .insert(item_id.clone());
        }
        Self { by_location }
    }

    /// Item IDs at exactly `location`, ascending.
    pub(crate) fn at<'a>(
        &'a self,
        location: &SharedItemLocation,
    ) -> impl DoubleEndedIterator<Item = &'a ItemInstanceId> + use<'a> {
        self.by_location
            .get(location)
            .into_iter()
            .flat_map(BTreeSet::iter)
    }
}

/// Copy-on-write item instance map with a derived location index.
#[derive(Default)]
pub(crate) struct ItemInstances {
    map: Arc<ItemInstanceMap>,
    index: OnceLock<Arc<ItemLocationIndex>>,
}

impl ItemInstances {
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.map, &other.map)
    }

    /// The location index for the current map, rebuilt at most once after
    /// each mutable access.
    pub(crate) fn location_index(&self) -> &ItemLocationIndex {
        self.index
            .get_or_init(|| Arc::new(ItemLocationIndex::build(&self.map)))
    }

    /// Item IDs at exactly `location`, ascending by ID.
    pub(crate) fn ids_at<'a>(
        &'a self,
        location: &SharedItemLocation,
    ) -> impl DoubleEndedIterator<Item = &'a ItemInstanceId> + use<'a> {
        self.location_index().at(location)
    }
}

impl Clone for ItemInstances {
    fn clone(&self) -> Self {
        Self {
            map: Arc::clone(&self.map),
            index: self.index.clone(),
        }
    }
}

impl std::fmt::Debug for ItemInstances {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.map.fmt(formatter)
    }
}

impl PartialEq for ItemInstances {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || *self.map == *other.map
    }
}

impl Eq for ItemInstances {}

impl From<ItemInstanceMap> for ItemInstances {
    fn from(map: ItemInstanceMap) -> Self {
        Self {
            map: Arc::new(map),
            index: OnceLock::new(),
        }
    }
}

impl Deref for ItemInstances {
    type Target = ItemInstanceMap;

    fn deref(&self) -> &ItemInstanceMap {
        &self.map
    }
}

impl DerefMut for ItemInstances {
    fn deref_mut(&mut self) -> &mut ItemInstanceMap {
        // Any mutable access may move, insert or remove an instance.
        self.index = OnceLock::new();
        Arc::make_mut(&mut self.map)
    }
}

impl<'a> IntoIterator for &'a ItemInstances {
    type Item = (&'a ItemInstanceId, &'a ItemInstance);
    type IntoIter = std::collections::btree_map::Iter<'a, ItemInstanceId, ItemInstance>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

impl<'a> IntoIterator for &'a mut ItemInstances {
    type Item = (&'a ItemInstanceId, &'a mut ItemInstance);
    type IntoIter = std::collections::btree_map::IterMut<'a, ItemInstanceId, ItemInstance>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}
