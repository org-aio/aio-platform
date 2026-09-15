use super::{
    identity,
    model::{NpmPackage, Repository},
    network, storage,
    validation::validate,
};
use crate::runtime::{
    RuntimeResponse,
    server::{RuntimeState, http_error::RuntimeError},
};
use axum::{Json, extract::State, http::HeaderMap};
use az_tool::{
    ToolManifest,
    publication::Publication,
    registration::{Documentation, Metadata},
};

pub(in crate::runtime::server::tools) async fn publish(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<Publication>,
) -> Result<Json<RuntimeResponse<ToolManifest>>, RuntimeError> {
    request.validate()?;
    let owner = state
        .config
        .delivery
        .as_ref()
        .map(|c| c.owner.as_str())
        .ok_or_else(|| RuntimeError::forbidden("平台未配置 GitHub 发布者"))?;
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| RuntimeError::unauthorized("缺少 GitHub Actions 发布身份"))?;
    let identity = identity::verify(token, owner)
        .await
        .map_err(|_| RuntimeError::unauthorized("GitHub Actions 发布身份无效"))?;
    let mut url = reqwest::Url::parse("https://registry.npmjs.org/")?;
    url.path_segments_mut()
        .unwrap()
        .push(&request.package)
        .push(&request.version);
    let package: NpmPackage = serde_json::from_slice(
        &network::bytes(url.as_str(), false, 1024 * 1024)
            .await
            .map_err(|_| RuntimeError::unavailable("npm 版本尚未就绪，请稍后重试"))?,
    )?;
    let manifest = validate(&request, &identity, &package)?;
    let repository: Repository = serde_json::from_slice(
        &network::bytes(
            &format!("https://api.github.com/repos/{}", identity.repository),
            true,
            128 * 1024,
        )
        .await?,
    )?;
    if identity.reference.starts_with("refs/heads/") {
        if identity.reference != format!("refs/heads/{}", repository.default_branch) {
            return Err(RuntimeError::forbidden("只允许默认分支自动上架"));
        }
    } else if identity.reference != format!("refs/tags/v{}", request.version)
        || !semver::Version::parse(&request.version)?.pre.is_empty()
    {
        return Err(RuntimeError::forbidden("正式发布必须使用相同版本标签"));
    }
    let git = format!("https://github.com/{}", identity.repository);
    let readme = network::bytes(
        &format!(
            "https://raw.githubusercontent.com/{}/{}/README.md",
            identity.repository, identity.sha
        ),
        false,
        256 * 1024,
    )
    .await;
    let doc = Documentation {
        metadata: Metadata {
            git: git.clone(),
            title: manifest.title.clone(),
            summary: manifest.summary.clone(),
        },
        link_base: format!("{git}/blob/{}/", identity.sha),
        image_base: format!(
            "https://raw.githubusercontent.com/{}/{}/",
            identity.repository, identity.sha
        ),
        readme: readme
            .as_ref()
            .ok()
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default(),
        error: readme
            .err()
            .map(|_| "README 暂时无法读取，可在详情页刷新".into()),
    };
    storage::publish(
        &state.store.pool,
        &manifest,
        &doc,
        &identity.sha,
        &package.dist.integrity,
    )
    .await?;
    Ok(Json(RuntimeResponse { data: manifest }))
}
