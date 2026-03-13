// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

macro_rules! env_required {
    ($name:literal) => {
        std::env::var($name).expect(concat!(
            "Environment variable `",
            $name,
            "` should be defined"
        ))
    };
}
pub(crate) use env_required;
