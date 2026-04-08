// prose-pod-server
//
// Copyright: 2026, Claude Sonnet 4.6
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::fmt;

pub struct AsMap<'a, K, V>(pub &'a [(K, V)]);

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for AsMap<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();
        for (k, v) in self.0 {
            map.entry(k, v);
        }
        map.finish()
    }
}
