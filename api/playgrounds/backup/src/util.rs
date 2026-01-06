// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

/// Casting with `as` can yield incorrect values and similar issues
/// happen with `clamp`. This function ensures no overflow happens.
pub fn saturating_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_saturating_i64_to_u64() {
        use crate::util::saturating_i64_to_u64;

        // Casting with `as` can yield incorrect values:
        assert_eq!(i64::MIN, -9223372036854775808);
        assert_eq!(i64::MIN as u64, 9223372036854775808);

        assert_eq!(u64::MIN, 0);
        assert_eq!(saturating_i64_to_u64(i64::MIN), 0);
        assert_eq!(i64::MAX, 9223372036854775807);
        assert_eq!(saturating_i64_to_u64(i64::MAX), 9223372036854775807);
    }
}
