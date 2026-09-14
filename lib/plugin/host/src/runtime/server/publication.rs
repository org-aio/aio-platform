use axum::{
    Extension, Json,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Request, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};

use super::{
    RuntimeState,
    http_error::RuntimeError,
    request_context::{
        PublisherContext, authenticate, authenticate_publisher, authorize_publish_target,
    },
};
use crate::runtime::{PublishState, PublishedPluginView, RuntimeResponse};

pub(super) async fn publish(
    State(state): State<RuntimeState>,
    Extension(publisher): Extension<PublisherContext>,
    body: Bytes,
) -> Result<Json<RuntimeResponse<PublishedPluginView>>, RuntimeError> {
    let package = tokio::task::spawn_blocking(move || PluginPackage::decode(&body))
        .await
        .map_err(|error| {
            RuntimeError::bad_request(format!("等待二进制插件包解码失败: {error}"))
        })??;
    let tenant_id = authorize_publish_target(publisher, &package.git, None)?;
    let staged = state.repository.stage_publish(&package).await?;
    state.store.save_package(&package).await?;
    let source_id = state
        .store
        .source_id(&package.git)
        .await?
        .unwrap_or(staged.source_id);
    let job = state
        .store
        .queue_publish_job(
            &tenant_id,
            &source_id,
            &package.git,
            &package.rev,
            staged.runtime,
            staged.pages.len(),
        )
        .await?;
    let _ = state
        .store
        .record_lifecycle_event(
            &tenant_id,
            &source_id,
            None,
            "publish-queued",
            "二进制包已保存 PostgreSQL，后台健康检查通过后原子激活",
        )
        .await;
    if job.state == PublishState::Queued {
        state.start_publish_job(job.clone());
    }
    Ok(Json(RuntimeResponse {
        data: PublishedPluginView {
            job_id: job.id,
            tenant_id,
            source_id,
            revision: package.rev,
            runtime: staged.runtime,
            page_count: staged.pages.len(),
            state: job.state,
            detail: job.detail,
        },
    }))
}

pub(super) async fn authorize_upload(
    State(state): State<RuntimeState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if let Err(error) = ensure_package_content_type(request.headers()) {
        return error.into_response();
    }
    let Ok(_permit) = state.publication_slots.clone().try_acquire_owned() else {
        return RuntimeError::unavailable("插件发布繁忙，请稍后重试").into_response();
    };
    match authenticate_publisher(&state, request.headers()).await {
        Ok(publisher) => request.extensions_mut().insert(publisher),
        Err(error) => return error.into_response(),
    };
    next.run(request).await
}

pub(super) async fn download_package(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
) -> Result<Response, RuntimeError> {
    authenticate(&state, &headers).await?;
    let native: Option<Vec<u8>> = if state.components.is_some() {
        sqlx::query_scalar("SELECT archive FROM component_versions WHERE digest=$1")
            .bind(&revision)
            .fetch_optional(&state.store.pool)
            .await?
    } else {
        None
    };
    let content_type = if native.is_some() {
        "application/vnd.aio.component+gzip"
    } else {
        PACKAGE_CONTENT_TYPE
    };
    let archive = if let Some(archive) = native {
        archive
    } else {
        state
            .store
            .downloadable_package(&revision)
            .await?
            .ok_or_else(|| RuntimeError::not_found("插件包不存在或尚未通过发布验证"))?
    };
    let mut response = archive.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{revision}.aio-plugin\""))
            .map_err(|_| RuntimeError::bad_request("插件包下载文件名无效"))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

fn ensure_package_content_type(headers: &HeaderMap) -> Result<(), RuntimeError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !content_type.is_some_and(|value| value.eq_ignore_ascii_case(PACKAGE_CONTENT_TYPE)) {
        return Err(RuntimeError::unsupported_media_type(
            "发布接口必须接收 application/vnd.aio.plugin+gzip 二进制包",
        ));
    }
    if headers.contains_key(header::CONTENT_ENCODING) {
        return Err(RuntimeError::unsupported_media_type(
            "插件包已有内置压缩，不接受 Content-Encoding",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_raw_binary_package_without_transport_encoding() {
        let mut headers = HeaderMap::new();
        assert!(ensure_package_content_type(&headers).is_err());
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert!(ensure_package_content_type(&headers).is_err());
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PACKAGE_CONTENT_TYPE),
        );
        assert!(ensure_package_content_type(&headers).is_ok());
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(ensure_package_content_type(&headers).is_err());
    }
}
