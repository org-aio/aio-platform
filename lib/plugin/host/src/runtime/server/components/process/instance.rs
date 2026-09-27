use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_runtime::bindings::aio::plugin::transport::{Header, Request, Response};
use std::{
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};
use uuid::Uuid;

use super::super::{Components, model::Description};
use super::{
    Processes,
    model::{Instance, Start, Stop},
};

impl Processes {
    pub async fn reconcile(&self) -> Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        let active: Vec<_> = self
            .instances
            .lock()
            .await
            .values()
            .map(|instance| instance.start.clone())
            .collect();
        let response = self
            .supervisor
            .post("http://localhost/bundles/reconcile")
            .json(&active)
            .send()
            .await?;
        ensure!(response.status().is_success(), "清理孤立 process 失败");
        Ok(())
    }
    pub fn client() -> Result<reqwest::Client> {
        let socket = std::env::var_os("AIO_PLUGIN_SUPERVISOR_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/run/aio-plugin-supervisor/supervisor.sock".into());
        Ok(reqwest::Client::builder()
            .unix_socket(socket)
            .no_proxy()
            .timeout(Duration::from_secs(90))
            .build()?)
    }

    pub fn new(components: Weak<Components>, root: PathBuf, supervisor: reqwest::Client) -> Self {
        Self {
            components,
            root,
            supervisor,
            instances: Default::default(),
            pending: Default::default(),
        }
    }

    pub async fn prepare(
        &self,
        source: Uuid,
        tenant: &str,
        bundle: &Bundle,
    ) -> Result<(Arc<Instance>, Description)> {
        let start = Start {
            source,
            tenant: tenant.into(),
            revision: bundle.digest.clone(),
        };
        self.stop_id(&start.id()).await?;
        let (config, jobs) = self
            .configure(
                &start,
                &bundle.verify()?,
                &self.root.join(start.id()),
                false,
            )
            .await?;
        let directory = self.root.join(start.id());
        tokio::fs::write(directory.join("bundle.aio-plugin"), bundle.encode()?).await?;
        let client = reqwest::Client::builder()
            .unix_socket(directory.join("runtime/service.sock"))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(35))
            .build()?;
        let instance = Arc::new(Instance {
            start: start.clone(),
            bundle: Arc::new(bundle.verify()?),
            token: config.ingress_token,
            client,
            resources: Arc::new(super::model::Resources {
                _jobs: jobs,
                _socket_alias: None,
            }),
        });
        let result = async {
            let response = self
                .supervisor
                .post("http://localhost/bundles/start")
                .json(&start)
                .send()
                .await
                .context("连接 process 监督器失败")?;
            ensure!(
                response.status().is_success(),
                "监督器拒绝启动 process（HTTP {}）",
                response.status()
            );
            for _ in 0..300 {
                if instance
                    .client
                    .get("http://localhost/health")
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
                {
                    let response = instance
                        .client
                        .get("http://localhost/aio/describe")
                        .send()
                        .await?;
                    ensure!(response.status().is_success(), "process 描述不可用");
                    let mut description: Description =
                        serde_json::from_slice(&read_body(response, 128 * 1024).await?)?;
                    description.process = true;
                    validate_description(&bundle.verify()?, &description)?;
                    return description.with_settings(&bundle.verify()?);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            anyhow::bail!("process 启动健康检查超时")
        }
        .await;
        match result {
            Ok(description) => Ok((instance, description)),
            Err(error) => {
                let _ = self.stop_id(&start.id()).await;
                Err(error)
            }
        }
    }

    pub async fn stop_id(&self, id: &str) -> Result<()> {
        let response = self
            .supervisor
            .post("http://localhost/bundles/stop")
            .json(&Stop { id: id.into() })
            .send()
            .await
            .context("停止 process 失败")?;
        ensure!(response.status().is_success(), "监督器拒绝停止 process");
        Ok(())
    }

    pub async fn stop(&self, source: Uuid, tenant: &str) -> Result<()> {
        let mut instances = self.instances.lock().await;
        if let Some(instance) = instances.get(&(source, tenant.into())) {
            self.stop_id(&instance.start.id()).await?;
        }
        instances.remove(&(source, tenant.into()));
        Ok(())
    }

    pub async fn activate(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        self.stop(source, tenant).await?;
        let (instance, _) = self.prepare(source, tenant, bundle).await?;
        self.instances
            .lock()
            .await
            .insert((source, tenant.into()), instance);
        Ok(())
    }

    pub async fn bundle(&self, source: Uuid, tenant: &str) -> Option<Arc<VerifiedBundle>> {
        self.instances
            .lock()
            .await
            .get(&(source, tenant.into()))
            .map(|instance| instance.bundle.clone())
    }

    pub async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        let instance = self
            .instances
            .lock()
            .await
            .get(&(source, tenant.into()))
            .cloned()
            .context("process 未激活")?;
        ensure!(
            instance.bundle.digest() == digest && context.tenant_id.as_deref() == Some(tenant),
            "process 活动版本或租户已变化"
        );
        let user = context.user_id.context("process 调用缺少用户")?;
        let components = self.components.upgrade().context("宿主已停止")?;
        sqlx::query("INSERT INTO component_process_actors(source_id,tenant_id,user_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(source).bind(tenant).bind(&user).execute(&components.pool).await?;
        let mut url = reqwest::Url::parse("http://localhost")?;
        url.set_path(&request.path);
        url.set_query(request.query.as_deref());
        let mut call = instance
            .client
            .request(reqwest::Method::from_bytes(request.method.as_bytes())?, url)
            .header("x-aio-token", &instance.token)
            .header("x-aio-tenant-id", tenant)
            .header("x-aio-user-id", user)
            .header("x-aio-context", context.request_id)
            .body(request.body);
        for header in request.headers {
            if ["content-type", "accept"].contains(&header.name.to_ascii_lowercase().as_str()) {
                call = call.header(header.name, header.value);
            }
        }
        let response = call.send().await.context("process 调用失败")?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                vec![Header {
                    name: "content-type".into(),
                    value: value.into(),
                }]
            })
            .unwrap_or_default();
        Ok(Response {
            status,
            headers,
            body: read_body(response, 8 * 1024 * 1024).await?,
        })
    }
}

pub(super) async fn read_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= limit, "process 响应超过配额");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) fn validate_description(
    bundle: &VerifiedBundle,
    description: &Description,
) -> Result<()> {
    ensure!(
        !description.label.is_empty()
            && description.label.len() <= 256
            && !description.pages.is_empty()
            && description.pages.len() <= 32,
        "process 页面描述无效"
    );
    let mut ids = std::collections::HashSet::new();
    for page in &description.pages {
        ensure!(
            !page.id.is_empty()
                && page.id.len() <= 128
                && page
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                && ids.insert(&page.id)
                && !page.label.is_empty()
                && page.label.len() <= 256,
            "process 页面标识无效"
        );
        ensure!(
            page.surface == "workspace" || (page.scene.is_none() && page.menu_path.is_empty()),
            "独立页面不应声明工作区导航"
        );
        ensure!(
            bundle.frontend(&page.entry).is_some(),
            "process 页面入口不属于整包"
        );
        ensure!(
            page.permission.as_ref().is_none_or(|p| bundle
                .manifest()
                .plugin
                .permissions
                .contains(p)),
            "process 页面权限未声明"
        );
        ensure!(
            ["workspace", "fullscreen", "account-entry", "account-menu"]
                .contains(&page.surface.as_str()),
            "process 页面类型无效"
        );
    }
    Ok(())
}
