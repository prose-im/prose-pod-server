// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

/// Just a [`u32`] which formats to string as an octal number.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Octal<const N_DIGITS: usize>(u32);

impl<'de, const N_DIGITS: usize> serde::Deserialize<'de> for Octal<N_DIGITS> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let n = u32::deserialize(deserializer)?;

        let max = (0o1 << 3 * N_DIGITS) - 1;
        if n > max {
            return Err(serde::de::Error::custom(format!(
                "Value must be ≤ {max:#o}"
            )));
        }

        Ok(Self(n))
    }
}

impl<const N_DIGITS: usize> std::fmt::Display for Octal<N_DIGITS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Octal::fmt(&self.0, f)
    }
}

impl<const N_DIGITS: usize> std::fmt::Debug for Octal<N_DIGITS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Octal::fmt(&self.0, f)
    }
}

impl<const N_DIGITS: usize> std::fmt::Octal for Octal<N_DIGITS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Octal::fmt(&self.0, f)
    }
}

impl<const N_DIGITS: usize> std::ops::Deref for Octal<N_DIGITS> {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
