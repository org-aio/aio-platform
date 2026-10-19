use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, ensure};
use axum::http::HeaderValue;

#[derive(Clone)]
pub(super) struct FrontendGrant {
    pub cookie: HeaderValue,
    pub session_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub page_id: String,
    pub source_id: String,
    pub activation_generation: String,
    pub revision: String,
    pub entry: String,
    pub frontend_path: String,
    pub assets: BTreeMap<String, String>,
    pub issued: Instant,
}

pub(super) struct FrontendAccess {
    pub packages:
        tokio::sync::Mutex<BTreeMap<String, Arc<super::frontend_package::FrontendPackage>>>,
    grants: Mutex<HashMap<String, FrontendGrant>>,
    pub origin: String,
    requests: Arc<tokio::sync::Semaphore>,
}

const HOST_GRANT_LIMIT: usize = 256;
const USER_GRANT_LIMIT: usize = 16;

impl FrontendAccess {
    pub fn new(origin: &str) -> Result<Self> {
        Ok(Self {
            packages: tokio::sync::Mutex::new(BTreeMap::new()),
            grants: Mutex::new(HashMap::new()),
            origin: super::frontend_document::public_origin(origin)?,
            requests: Arc::new(tokio::sync::Semaphore::new(32)),
        })
    }

    pub fn active_revisions(&self) -> Result<std::collections::BTreeSet<String>> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?;
        grants.retain(|_, grant| grant.issued.elapsed() < Duration::from_secs(1800));
        Ok(grants
            .values()
            .map(|grant| grant.revision.clone())
            .collect())
    }

    pub fn request_slot(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.requests
            .clone()
            .try_acquire_owned()
            .context("前端请求超过宿主并发配额")
    }

    /// 同一会话、同一页面、同一插件版本的挂载复用同一个票据。
    ///
    /// 票据出现在资源 URL 里；每次打开都换票据会让 CSS、字体这类由浏览器直接加载的
    /// 资源全部变成新地址，HTTP 缓存必然失效并重复下载。复用票据后地址按版本稳定，
    /// 插件升级会改变 revision 从而自然换取新票据。
    pub fn issue(&self, grant: FrontendGrant) -> Result<String> {
        ensure!(grant.cookie.as_bytes().len() <= 8192, "会话 Cookie 过大");
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?;
        grants.retain(|_, grant| grant.issued.elapsed() < Duration::from_secs(1800));
        let reusable = grants.iter().find(|(_, existing)| {
            existing.session_id == grant.session_id
                && existing.user_id == grant.user_id
                && existing.tenant_id == grant.tenant_id
                && existing.page_id == grant.page_id
                && existing.source_id == grant.source_id
                && existing.revision == grant.revision
                && existing.activation_generation == grant.activation_generation
                && existing.entry == grant.entry
                && existing.frontend_path == grant.frontend_path
        });
        if let Some((token, _)) = reusable {
            let token = token.clone();
            // 续期到本次挂载时间，避免长时间停留的页面因票据过期而重新换地址。
            if let Some(existing) = grants.get_mut(&token) {
                existing.issued = Instant::now();
            }
            return Ok(token);
        }
        if grants.len() >= HOST_GRANT_LIMIT {
            let oldest = grants
                .iter()
                .min_by_key(|(_, existing)| existing.issued)
                .map(|(token, _)| token.clone());
            if let Some(token) = oldest {
                grants.remove(&token);
            }
        }
        if grants
            .values()
            .filter(|existing| existing.user_id == grant.user_id)
            .count()
            >= USER_GRANT_LIMIT
        {
            let oldest = grants
                .iter()
                .filter(|(_, existing)| existing.user_id == grant.user_id)
                .min_by_key(|(_, existing)| existing.issued)
                .map(|(token, _)| token.clone());
            if let Some(token) = oldest {
                grants.remove(&token);
            }
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        grants.insert(token.clone(), grant);
        Ok(token)
    }

    pub fn get(&self, token: &str) -> Result<FrontendGrant> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?;
        grants.retain(|_, grant| grant.issued.elapsed() < Duration::from_secs(1800));
        grants
            .get(token)
            .cloned()
            .context("前端挂载凭证无效或已过期")
    }

    pub fn remove(&self, token: &str) -> Result<()> {
        self.grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?
            .remove(token);
        Ok(())
    }

    pub fn renew(&self, token: &str) -> Result<()> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?;
        let grant = grants.get_mut(token).context("前端挂载已释放")?;
        ensure!(
            grant.issued.elapsed() < Duration::from_secs(1800),
            "前端挂载已过期"
        );
        grant.issued = Instant::now();
        Ok(())
    }
}
