// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub async fn print_all_objects(
    s3_client: &s3::Client,
    bucket: impl Into<String>,
) -> Result<(), anyhow::Error> {
    tracing::info!("Listing all objects…");
    let response = s3_client
        .list_object_versions()
        .bucket(bucket)
        .send()
        .await?;

    for marker in response.delete_markers() {
        tracing::info!(
            "delete marker: key={} version={}",
            marker.key().unwrap(),
            marker.version_id().unwrap()
        );
    }

    for version in response.versions() {
        tracing::info!(
            "version: key={} version={} latest={:?}",
            version.key().unwrap(),
            version.version_id().unwrap(),
            version.is_latest()
        );
    }

    Ok(())
}
