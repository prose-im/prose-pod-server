// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use secrecy::SecretSlice;
use sha2::{Digest, Sha256};

use crate::app_config::{
    VendorAnalyticsAcquisitionConfig, VendorAnalyticsConfig, VendorAnalyticsUsageConfig,
};

/// To future-proof the Prose Pod Server’s code and avoid having to make change
/// every time we want to add an anallytics event in a client app, the
/// Prose Pod Server does not parse analytics events. It reads it as JSON
/// and only overrides what it knows about.
///
/// FYI currently events look like:
///
/// ```json
/// {
///   "name": "account:signin",
///   "data": {
///     "credential_type": "password"
///   },
///   "origin": {
///     "app": {
///       "name": "prose-app-web",
///       "version": "1.0.1",
///       "platform": "macos-aarch64"
///     },
///     "pod": {
///       "domain_hash": "f4ce232118cee9d2",
///       "user_hash": "19a124bd2f89edfe"
///     }
///   }
/// }
/// ```
pub type AnalyticsEvent = json::Value;

pub(crate) fn process_event(
    mut event: AnalyticsEvent,
    config: &VendorAnalyticsConfig,
    pod_domain_value: &str,
    user_count: u64,
    server_salt: &SecretSlice<u8>,
) -> Option<AnalyticsEvent> {
    // Deconstruct and map names cleverly so
    // `rustc` ensures we don’t forget one key.
    let VendorAnalyticsConfig {
        enabled,
        preset: _,
        presets: _,
        min_cohort_size,
        usage:
            VendorAnalyticsUsageConfig {
                enabled: usage,
                meta_user_count,
                pod_version,
                user_app_version,
                user_lang,
                user_platform,
            },
        acquisition:
            VendorAnalyticsAcquisitionConfig {
                enabled: acquisition,
                pod_domain,
            },
    } = config;

    if !enabled {
        return None;
    }

    if let Some(min_cohort_size) = min_cohort_size {
        if user_count < (*min_cohort_size as u64) {
            return None;
        }
    }

    macro_rules! get_enabled {
        ($category:ident.$key:ident) => {
            if *$category && $key.enabled {
                Some($key)
            } else {
                None
            }
        };
    }
    macro_rules! enabled {
        ($category:ident.$key:ident) => {
            get_enabled!($category.$key) != None
        };
    }
    macro_rules! disabled {
        ($category:ident.$key:ident) => {
            get_enabled!($category.$key) == None
        };
    }
    macro_rules! remove {
        ($key:literal from $([$path:literal])+) => {
            if let Some(object) = event$([$path])+.as_object_mut() {
                object.remove($key);
            }
        };
    }

    let origin_name = event["origin"]["name"]
        .as_str()
        // COMPAT: Compatibility with original
        .or_else(|| event["origin"]["app"]["name"].as_str())
        .map(ToOwned::to_owned);

    if disabled!(usage.pod_version) {
        remove!("version" from ["origin"]["pod"]);
    }

    if disabled!(usage.user_app_version) {
        if let Some(origin_name) = origin_name {
            if origin_name.starts_with("prose-app-") {
                remove!("version" from ["origin"]["app"]);
                remove!("version" from ["origin"]);
            }
        }
    }

    if let Some(user_platform) = get_enabled!(usage.user_platform) {
        if let Some(platform) = event["origin"]["app"]["platform"].as_str() {
            let platform = platform.to_owned();

            if let Some(ref deny_list) = user_platform.deny_list {
                if deny_list.contains(platform.as_str()) {
                    remove!("platform" from ["origin"]["app"]);
                }
            };

            if let Some(ref allow_list) = user_platform.allow_list {
                if !allow_list.contains(platform.as_str()) {
                    remove!("platform" from ["origin"]["app"]);
                }
            };
        }
    } else {
        remove!("platform" from ["origin"]["app"]);
    }

    if let Some(user_lang) = get_enabled!(usage.user_lang) {
        if let Some(max_locales) = user_lang.max_locales {
            if let Some(array) = event["origin"]["app"]["user_preferred_locales"].as_array_mut() {
                array.truncate(max_locales)
            }
        }
    } else {
        remove!("user_preferred_locales" from ["origin"]["app"]);
    }

    if enabled!(acquisition.pod_domain) {
        event["origin"]["pod"]["identifiable"]["domain"] = json::to_value(pod_domain_value)
            .expect("`pod_domain` should be a JSON-encodable string");
    }

    if enabled!(usage.meta_user_count) {
        let (min, max) = user_count_range(user_count);

        // Expose min and max for easier analysis.
        // SAFETY: `u64` cannot fail to encode as
        event["origin"]["pod"]["user_count_min"] = json::Number::from(min).into();
        event["origin"]["pod"]["user_count_max"] = json::Number::from(max).into();

        // Expose range in different formats for easier graphs.
        // NOTE: For other formats, compute on the analytics side.
        event["origin"]["pod"]["user_count_range"] = format!("{min}–{max}").into();
        event["origin"]["pod"]["user_count_range_open"] = format!("{min}+").into();
    }

    // Anonymize user with random server-side salt.
    if let Some(user_hash) = event["origin"]["pod"]["user_hash"].as_str() {
        let user_hash = anonymize(user_hash.as_bytes(), server_salt);

        event["origin"]["pod"]["user_hash"] = json::to_value(user_hash).unwrap();
    };

    // Add `proxied` flag for our analytics system.
    event["proxied"] = json::Value::Bool(true);

    Some(event)
}

/// 0–4, 5–9, 10–19, 20–49, 50–99, 100–199, 200–299, etc. (by hundreds above).
fn user_count_range(user_count: u64) -> (u64, u64) {
    match user_count {
        0..=4 => (0, 4),
        5..=9 => (5, 9),
        10..=19 => (10, 19),
        20..=49 => (20, 49),
        50..=99 => (50, 99),
        n => {
            let hundreds_floor = n - (n % 100);
            (hundreds_floor, hundreds_floor + 99)
        }
    }
}

fn anonymize(user_hash: &[u8], server_salt: &SecretSlice<u8>) -> String {
    use secrecy::ExposeSecret as _;

    debug_assert_eq!(user_hash.len(), 16);

    let anonymous_hash = {
        let mut hasher = Sha256::new();
        hasher.update(user_hash);
        hasher.update(server_salt.expose_secret());
        hasher.finalize()
    };
    debug_assert!(anonymous_hash.len() > 16);

    let ref user_hash = hex::encode(anonymous_hash)[..16];
    debug_assert_eq!(user_hash.len(), 16);

    user_hash.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_count_range() {
        assert_eq!(user_count_range(0), (0, 4), "0");
        assert_eq!(user_count_range(4), (0, 4), "4");
        assert_eq!(user_count_range(5), (5, 9), "5");
        assert_eq!(user_count_range(9), (5, 9), "9");
        assert_eq!(user_count_range(100), (100, 199), "100");
        assert_eq!(user_count_range(199), (100, 199), "199");
        assert_eq!(user_count_range(200), (200, 299), "200");
        assert_eq!(user_count_range(299), (200, 299), "299");
        assert_eq!(user_count_range(300), (300, 399), "300");
    }

    /// Using `Write::write` on a SHA-256 hasher can fail. `Digest::update`
    /// does not, but it’s unclear if it *replaces* the hasher’s state or if
    /// it *adds* (“writes”) to it. This test clarifies it.
    #[test]
    fn test_sha256_update_not_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write as _;

        let hash_write = {
            let mut hasher = Sha256::new();
            hasher.write("ab".as_bytes())?;
            hasher.write("cd".as_bytes())?;
            hasher.finalize()
        };
        let hash_update_intermediary = {
            let mut hasher = Sha256::new();
            hasher.update("ab");
            hasher.finalize()
        };
        let hash_update = {
            let mut hasher = Sha256::new();
            hasher.update("ab");
            hasher.update("cd");
            hasher.finalize()
        };

        assert_eq!(hash_update, hash_write);
        assert_ne!(hash_update, hash_update_intermediary);

        Ok(())
    }

    #[test]
    fn test_anonymize_user_id() {
        let server_salt: SecretSlice<u8> = crate::util::random_bytes::<256>().to_vec().into();

        fn is_lower_hex(s: &str) -> bool {
            s.len() % 2 == 0 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }

        let user1: &[u8] = "d1c2e691c1a0f256".as_bytes();
        let hash1: String = anonymize(user1, &server_salt);
        assert!(is_lower_hex(hash1.as_str()));

        let user2: &[u8] = "d4fb3752f9a55da7".as_bytes();
        let hash2: String = anonymize(user2, &server_salt);
        assert!(is_lower_hex(hash2.as_str()));

        // Ensure `user_hash` is used (not just `server_hash`).
        assert_ne!(hash1, hash2);
    }
}
