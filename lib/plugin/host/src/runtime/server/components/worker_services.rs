//! 设备只调用已安装插件明确开放的能力入口，宿主注入所有者身份。
use super::super::{RuntimeState, http_error::RuntimeError};
use crate::{
    generated::worker::{controller::device, model::DeviceIdentity},
    runtime::RuntimeResponse,
};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use az_plugin_contract::RequestContext;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const CAPABILITY: &str = "skills.sync";

pub(super) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/workers/self/skills", put(configure))
        .route("/api/runtime/workers/services", get(services))
        .route(
            "/api/runtime/workers/services/{source}/{capability}",
            post(invoke),
        )
        .layer(DefaultBodyLimit::max(3 * 1024 * 1024))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Access {
    enabled: bool,
}

// 只有本机持有设备凭据的 worker 能主动开通同步，不需要重新配对或保存账号密码。
async fn configure(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(access): Json<Access>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let identity = device(&state, &headers).await?;
    sqlx::query("UPDATE worker_devices SET capabilities=(SELECT coalesce(jsonb_agg(value),'[]'::jsonb) FROM jsonb_array_elements(capabilities) WHERE value<>to_jsonb($2::text)) || CASE WHEN $3 THEN jsonb_build_array($2::text) ELSE '[]'::jsonb END WHERE id=$1 AND state='active'")
        .bind(identity.id).bind(CAPABILITY).bind(access.enabled).execute(&state.store.pool).await?;
    Ok(Json(RuntimeResponse { data: () }))
}

fn allowed(identity: &DeviceIdentity, capability: &str) -> anyhow::Result<()> {
    ensure!(
        capability == CAPABILITY && identity.capabilities.iter().any(|c| c == capability),
        "设备尚未开通该能力"
    );
    ensure!(
        std::env::var("AIO_PROCESS_WORKER_CAPABILITIES")
            .unwrap_or_default()
            .split(',')
            .any(|c| c.trim() == capability),
        "宿主尚未开通该能力"
    );
    Ok(())
}

async fn services(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<Value>>, RuntimeError> {
    let identity = device(&state, &headers).await?;
    allowed(&identity, CAPABILITY).map_err(|_| RuntimeError::forbidden("Skill 同步尚未开启"))?;
    let components = state.components()?;
    let sources: Vec<Uuid> = sqlx::query_scalar("SELECT source_id FROM component_installations WHERE tenant_id=$1 AND enabled ORDER BY source_id")
        .bind(&identity.tenant).fetch_all(&components.pool).await?;
    let mut result = Vec::new();
    for source in sources {
        let bundle = components.bundle(source, &identity.tenant).await?;
        if bundle
            .manifest()
            .plugin
            .runtime
            .process
            .as_ref()
            .is_some_and(|p| p.worker_capabilities.iter().any(|c| c == CAPABILITY))
        {
            let (_, _, description) = components.description(&identity.tenant, source).await?;
            result.push(json!({"id":source,"label":description.label,"capabilities":[CAPABILITY]}));
        }
    }
    Ok(Json(RuntimeResponse {
        data: json!(result),
    }))
}

async fn invoke(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((source, capability)): Path<(Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<Response, RuntimeError> {
    let identity = device(&state, &headers).await?;
    allowed(&identity, &capability).map_err(|_| RuntimeError::forbidden("设备能力未授权"))?;
    let components = state.components()?;
    let installed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations WHERE tenant_id=$1 AND source_id=$2 AND enabled)")
        .bind(&identity.tenant).bind(source).fetch_one(&components.pool).await?;
    if !installed {
        return Err(RuntimeError::forbidden("目标插件未启用"));
    }
    let bundle = components.bundle(source, &identity.tenant).await?;
    let process = bundle
        .manifest()
        .plugin
        .runtime
        .process
        .as_ref()
        .context("目标插件不是设备服务")?;
    if !process.worker_capabilities.contains(&capability) {
        return Err(RuntimeError::forbidden("插件未声明此设备能力"));
    }
    let context = RequestContext {
        tenant_id: Some(identity.tenant.clone()),
        user_id: Some(identity.user),
        session_id: None,
        request_id: format!("worker:{}", identity.id),
    };
    let request = super::model::Request {
        method: "POST".into(),
        path: format!("/worker/{capability}"),
        query: None,
        headers: vec![super::model::Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: serde_json::to_vec(&body)?,
    };
    let response = components
        .handle(
            source,
            &identity.tenant,
            bundle.digest(),
            request.try_into()?,
            context,
        )
        .await?;
    let value: Value = serde_json::from_slice(&response.body)?;
    Ok((
        StatusCode::from_u16(response.status)?,
        Json(RuntimeResponse { data: value }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::worker::model::PairRequest;
    use crate::identity::{IdentityProvider, SessionContext};
    use std::sync::Arc;

    struct Identity;
    #[async_trait::async_trait]
    impl IdentityProvider for Identity {
        async fn authenticate(&self, _: &HeaderMap) -> anyhow::Result<Option<SessionContext>> {
            Ok(None)
        }
        async fn can_publish(&self, _: &SessionContext) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn member_active(&self, _: &str, _: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn session_active(&self, _: &str, _: &str, _: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
    }
    #[tokio::test]
    #[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
    async fn skill_grants_require_device_identity_preserve_other_capabilities_and_revoke()
    -> anyhow::Result<()> {
        let database = std::env::var("AIO_TEST_DATABASE_URL")?;
        let admin = sqlx::PgPool::connect(&database).await?;
        let schema = format!("skill_worker_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let mut database = reqwest::Url::parse(&database)?;
        database
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let root = tempfile::tempdir()?;
        let state = RuntimeState::isolated_admin_test(
            Arc::new(Identity),
            database.as_str(),
            "http://127.0.0.1:1",
            root.path(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let pair = state
            .workers
            .pair(PairRequest {
                label: "device".into(),
                platform: "darwin".into(),
                capabilities: vec!["space.scan".into()],
            })
            .await
            .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let session = SessionContext {
            session_id: "s".into(),
            tenant_id: "tenant".into(),
            user_id: "owner".into(),
            account: String::new(),
            display_name: String::new(),
            tenant_label: String::new(),
            permissions: vec![],
        };
        state.workers.approve(&session, &pair.code).await?;
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {}", pair.token).parse()?);
        assert!(
            configure(
                State(state.clone()),
                HeaderMap::new(),
                Json(Access { enabled: true })
            )
            .await
            .is_err()
        );
        let _ = configure(
            State(state.clone()),
            headers.clone(),
            Json(Access { enabled: true }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let _ = configure(
            State(state.clone()),
            headers.clone(),
            Json(Access { enabled: true }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let identity = state.workers.identity(&pair.token).await?;
        assert_eq!(identity.capabilities, vec!["space.scan", "skills.sync"]);
        assert!(allowed(&identity, "shell.execute").is_err());
        let _ = configure(
            State(state.clone()),
            headers.clone(),
            Json(Access { enabled: false }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        let identity = state.workers.identity(&pair.token).await?;
        assert_eq!(identity.capabilities, vec!["space.scan"]);
        assert!(
            services(State(state.clone()), headers.clone())
                .await
                .is_err()
        );
        state.workers.revoke(&session, &pair.device_id).await?;
        assert!(
            configure(State(state), headers, Json(Access { enabled: true }))
                .await
                .is_err()
        );
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .map_err(|_| anyhow::anyhow!("设备同步测试调用失败"))?;
        Ok(())
    }
}
