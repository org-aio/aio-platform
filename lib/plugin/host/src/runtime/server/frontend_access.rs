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

    pub fn issue(&self, grant: FrontendGrant) -> Result<String> {
        ensure!(grant.cookie.as_bytes().len() <= 8192, "会话 Cookie 过大");
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("前端挂载记录不可用"))?;
        grants.retain(|_, grant| grant.issued.elapsed() < Duration::from_secs(1800));
        ensure!(grants.len() < 256, "前端挂载数量超过宿主配额");
        ensure!(
            grants
                .values()
                .filter(|existing| existing.user_id == grant.user_id)
                .count()
                < 16,
            "当前用户的前端挂载数量超过配额"
        );
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
