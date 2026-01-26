// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::app_config::{
    VendorAnalyticsAcquisitionConfig, VendorAnalyticsConfig, VendorAnalyticsUsageConfig,
};

/// To future-proof the Prose Pod Server’s code and avoid having to make change
/// every time we want to add an anallytics event in a client app, the
/// Prose Pod Server does not parse analytics events. It reads it as JSON
/// and only overrides what it knows about.
pub type AnalyticsEvent = json::Value;

fn process_event(
    mut event: AnalyticsEvent,
    config: &VendorAnalyticsConfig,
    pod_domain_value: &str,
    user_count: u64,
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
        event["origin"]["pod"]["user_count"] = json::to_value(user_count).unwrap();
    }

    // Add `proxied` flag for our analytics system.
    event["proxied"] = json::Value::Bool(true);

    Some(event)
}
