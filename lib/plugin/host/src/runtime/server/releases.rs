use super::{RuntimeState, http_error::RuntimeError, request_context::authenticate};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use az_plugin_development::PublishedRelease;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseQuery {
    git: String,
}

/// 开发依赖只能选择已通过发布验证的版本，不能读取待验证构建。
pub(super) async fn list(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<ReleaseQuery>,
) -> Result<Json<Vec<PublishedRelease>>, RuntimeError> {
    authenticate(&state, &headers).await?;
    let git = az_plugin_package::normalize_git_source(&query.git)?;
    let rows = sqlx::query_as::<_, (String, String, String, String)>("SELECT p.version,p.source_revision,p.revision,p.manifest_toml FROM plugin_packages p WHERE p.git=$1 AND p.source_revision IS NOT NULL AND EXISTS(SELECT 1 FROM plugin_revisions r JOIN plugin_sources s ON s.id=r.source_id WHERE r.revision=p.revision AND s.git=p.git) ORDER BY p.created_at DESC LIMIT 100")
        .bind(&git).fetch_all(&state.store.pool).await?;
    let mut releases = rows
        .into_iter()
        .map(|(version, source_sha, digest, manifest)| PublishedRelease {
            git: git.clone(),
            version,
            source_sha,
            digest,
            manifest,
            abi: 1,
        })
        .collect::<Vec<_>>();
    if state.components.is_some() {
        let archives: Vec<Vec<u8>> = sqlx::query_scalar("SELECT v.archive FROM component_versions v JOIN component_sources s ON s.id=v.source_id JOIN component_publications p ON p.source_id=s.id WHERE s.git=$1 ORDER BY v.created_at DESC LIMIT 100")
            .bind(&git).fetch_all(&state.store.pool).await?;
        for archive in archives {
            let bundle = az_plugin_bundle::Bundle::decode(&archive)?;
            releases.push(PublishedRelease {
                git: git.clone(),
                version: bundle.version,
                source_sha: bundle.commit,
                digest: bundle.digest,
                manifest: bundle.manifest,
                abi: 2,
            });
        }
    }
    Ok(Json(releases))
}
