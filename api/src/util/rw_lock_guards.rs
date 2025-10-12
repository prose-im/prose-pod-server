// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use tokio::sync::RwLockReadGuard;

/// A [`tokio::sync::RwLockReadGuard`], but only giving access to nested data.
///
/// This is useful for example when we want to return a value from a
/// `RwLock`ed `HashMap` without cloning to control concurrency.
pub struct OptionRwLockReadGuard<'a, Base, ProjectedValue> {
    _guard: RwLockReadGuard<'a, Base>,
    value: Option<ProjectedValue>,
}

impl<'a, B, P> OptionRwLockReadGuard<'a, B, P> {
    /// API similar to [`RwLockReadGuard::map`].
    pub fn map(
        guard: RwLockReadGuard<'a, B>,
        map: impl FnOnce(&RwLockReadGuard<'a, B>) -> Option<P>,
    ) -> Self {
        Self {
            value: map(&guard),
            _guard: guard,
        }
    }
}

impl<'a, B, P> std::ops::Deref for OptionRwLockReadGuard<'a, B, P> {
    type Target = Option<P>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
