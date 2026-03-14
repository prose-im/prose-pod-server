// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

macro_rules! env_required {
    ($name:expr) => {
        std::env::var($name).expect(&format!(
            "Environment variable `{}` should be defined",
            $name
        ))
    };
}
pub(crate) use env_required;
