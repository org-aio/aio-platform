use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::InvocationScope;
use sqlx::{Postgres, Transaction};
use wasmtime::{StoreLimits, StoreLimitsBuilder};

use crate::{
    InvocationResources,
    bindings::aio::plugin::{
        cryptography, database, host, management, metadata, storage, transport,
    },
    database_query,
};

pub(crate) struct InvocationState {
    pub scope: InvocationScope,
    pub resources: InvocationResources,
    pub limits: StoreLimits,
    pub executing: bool,
    pub log_events: u32,
    transactions: HashMap<u32, Transaction<'static, Postgres>>,
    next_transaction: u32,
    wasi: wasmtime_wasi::WasiCtx,
    table: wasmtime::component::ResourceTable,
}

impl InvocationState {
    pub fn new(scope: InvocationScope, resources: InvocationResources) -> Self {
        let mut table = wasmtime::component::ResourceTable::new();
        table.set_max_capacity(128);
        Self {
            scope,
            resources,
            limits: StoreLimitsBuilder::new()
                .memory_size(128 * 1024 * 1024)
                .instances(128)
                .tables(32)
                .table_elements(100_000)
                .memories(8)
                .trap_on_grow_failure(true)
                .build(),
            executing: false,
            log_events: 0,
            transactions: HashMap::new(),
            next_transaction: 0,
            wasi: wasmtime_wasi::WasiCtxBuilder::new()
                .max_random_size(4096)
                .allow_tcp(false)
                .allow_udp(false)
                .allow_ip_name_lookup(false)
                .stdout(wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(4096))
                .stderr(wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(4096))
                .build(),
            table,
        }
    }

    pub async fn cleanup(&mut self) -> Result<()> {
        self.executing = false;
        let transactions = std::mem::take(&mut self.transactions);
        let mut failures = Vec::new();
        for (_, transaction) in transactions {
            if let Err(error) = transaction.rollback().await {
                failures.push(error.to_string());
            }
        }
        ensure!(
            failures.is_empty(),
            "清理插件事务失败: {}",
            failures.join("; ")
        );
        Ok(())
    }

    pub(crate) fn permit(&self, granted: bool) -> Result<()> {
        ensure!(self.executing && granted, "当前调用未授予该宿主能力");
        Ok(())
    }

    async fn failed_transaction(&mut self, id: u32) {
        if let Some(transaction) = self.transactions.remove(&id) {
            let _ = transaction.rollback().await;
        }
    }
}

impl cryptography::Host for InvocationState {
    async fn seal(
        &mut self,
        purpose: String,
        plaintext: Vec<u8>,
    ) -> wasmtime::Result<Result<Vec<u8>, String>> {
        Ok(wire((|| {
            self.permit(self.scope.grants.cryptography)?;
            self.resources
                .keyring
                .as_ref()
                .context("宿主加密密钥未绑定")?
                .seal(&self.scope, &purpose, &plaintext)
        })()))
    }

    async fn open(
        &mut self,
        purpose: String,
        ciphertext: Vec<u8>,
    ) -> wasmtime::Result<Result<Vec<u8>, String>> {
        Ok(wire((|| {
            self.permit(self.scope.grants.cryptography)?;
            self.resources
                .keyring
                .as_ref()
                .context("宿主加密密钥未绑定")?
                .open(&self.scope, &purpose, &ciphertext)
        })()))
    }
}

impl transport::Host for InvocationState {}
impl metadata::Host for InvocationState {}

impl wasmtime_wasi::WasiView for InvocationState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl host::Host for InvocationState {
    async fn context(&mut self) -> wasmtime::Result<host::RequestContext> {
        let context = &self.scope.context;
        Ok(host::RequestContext {
            tenant_id: context.tenant_id.clone(),
            user_id: context.user_id.clone(),
            session_id: context.session_id.clone(),
            request_id: context.request_id.clone(),
        })
    }

    async fn now(&mut self) -> wasmtime::Result<u64> {
        Ok(SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .try_into()?)
    }

    async fn random(&mut self, length: u32) -> wasmtime::Result<Result<Vec<u8>, String>> {
        let result = (|| {
            ensure!(length <= 4096, "随机字节数超过配额");
            let mut bytes = vec![0; length as usize];
            getrandom::fill(&mut bytes)
                .map_err(|error| anyhow::anyhow!("读取安全随机源失败: {error}"))?;
            Ok(bytes)
        })();
        Ok(wire(result))
    }

    async fn log(&mut self, message: String) -> wasmtime::Result<()> {
        wasmtime::error::ensure!(message.len() <= 4096, "日志超过配额");
        wasmtime::error::ensure!(self.log_events < 64, "日志条数超过配额");
        self.log_events += 1;
        tracing::info!(source = %self.scope.source_id, tenant = ?self.scope.context.tenant_id, revision = %self.scope.revision, "{message}");
        Ok(())
    }

    async fn authorize(&mut self, permission: String) -> wasmtime::Result<Result<bool, String>> {
        let result = async move {
            self.permit(true)?;
            ensure!(permission.len() <= 256, "权限声明过长");
            self.resources
                .services
                .as_ref()
                .context("授权服务未绑定")?
                .authorize(&self.scope, &permission)
                .await
        }
        .await;
        Ok(wire(result))
    }
}

impl database::Host for InvocationState {
    async fn begin(&mut self) -> wasmtime::Result<Result<u32, String>> {
        let result = async {
            self.permit(self.scope.grants.database)?;
            ensure!(self.transactions.len() < 4, "事务数超过配额");
            let id = self
                .next_transaction
                .checked_add(1)
                .context("事务句柄耗尽")?;
            self.next_transaction = id;
            let transaction = self
                .resources
                .database
                .as_ref()
                .context("数据库未绑定")?
                .begin()
                .await?;
            self.transactions.insert(id, transaction);
            Ok(id)
        }
        .await;
        Ok(wire(result))
    }

    async fn query(
        &mut self,
        transaction: u32,
        statement: String,
        parameters: Vec<database::Value>,
    ) -> wasmtime::Result<Result<database::Rows, String>> {
        let result = async {
            self.permit(self.scope.grants.database)?;
            let tx = self
                .transactions
                .get_mut(&transaction)
                .context("事务句柄无效或已经结束")?;
            tokio::time::timeout(
                Duration::from_secs(4),
                database_query::query(tx, &statement, parameters),
            )
            .await
            .context("数据库请求超时")?
        }
        .await;
        if result.is_err() {
            self.failed_transaction(transaction).await;
        }
        Ok(wire(result))
    }

    async fn execute(
        &mut self,
        transaction: u32,
        statement: String,
        parameters: Vec<database::Value>,
    ) -> wasmtime::Result<Result<u64, String>> {
        let result = async {
            self.permit(self.scope.grants.database)?;
            let tx = self
                .transactions
                .get_mut(&transaction)
                .context("事务句柄无效或已经结束")?;
            tokio::time::timeout(
                Duration::from_secs(4),
                database_query::execute(tx, &statement, parameters),
            )
            .await
            .context("数据库请求超时")?
        }
        .await;
        if result.is_err() {
            self.failed_transaction(transaction).await;
        }
        Ok(wire(result))
    }

    async fn finish(
        &mut self,
        transaction: u32,
        commit: bool,
    ) -> wasmtime::Result<Result<(), String>> {
        let result = async {
            self.permit(self.scope.grants.database)?;
            let tx = self
                .transactions
                .remove(&transaction)
                .context("事务句柄无效或已经结束")?;
            if commit {
                tx.commit().await.context("提交插件事务失败")
            } else {
                tx.rollback().await.context("回滚插件事务失败")
            }
        }
        .await;
        Ok(wire(result))
    }
}

impl storage::Host for InvocationState {
    async fn read(&mut self, path: String) -> wasmtime::Result<Result<Vec<u8>, String>> {
        let result = async move {
            self.permit(self.scope.grants.storage)?;
            self.resources
                .storage
                .as_ref()
                .context("对象存储未绑定")?
                .read(&path)
                .await
        }
        .await;
        Ok(wire(result))
    }

    async fn write(
        &mut self,
        path: String,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), String>> {
        let result = async move {
            self.permit(self.scope.grants.storage)?;
            self.resources
                .storage
                .as_ref()
                .context("对象存储未绑定")?
                .write(&path, &bytes)
                .await
        }
        .await;
        Ok(wire(result))
    }

    async fn remove(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        let result = async move {
            self.permit(self.scope.grants.storage)?;
            self.resources
                .storage
                .as_ref()
                .context("对象存储未绑定")?
                .remove(&path)
                .await
        }
        .await;
        Ok(wire(result))
    }
}

impl management::Host for InvocationState {
    async fn invoke(
        &mut self,
        request: transport::Request,
    ) -> wasmtime::Result<Result<transport::Response, String>> {
        let result = async move {
            self.permit(self.scope.grants.management)?;
            ensure!(
                self.scope.context.user_id.is_some() && self.scope.context.tenant_id.is_some(),
                "匿名请求不能调用插件管理"
            );
            ensure!(request.body.len() <= 32 * 1024 * 1024, "管理请求超过配额");
            // 组件 authorize 只处理组件权限；管理服务必须实时核验宿主身份和管理授权。
            self.resources
                .services
                .as_ref()
                .context("管理服务未绑定")?
                .manage(&self.scope, request.into())
                .await
                .map(Into::into)
        }
        .await;
        Ok(wire(result))
    }
}

impl crate::bindings::aio::plugin::enhancements::Host for InvocationState {
    async fn invoke(
        &mut self,
        path: String,
        input: String,
    ) -> wasmtime::Result<Result<String, String>> {
        let result = async move {
            self.permit(self.scope.grants.enhancements.contains(&path))?;
            ensure!(input.len() <= 64 * 1024, "增强输入超过配额");
            az_plugin_contract::validate_enhancement_path(&path)?;
            let request = az_plugin_contract::EnhancementRequest {
                path,
                input: serde_json::from_str(&input).context("增强输入必须是 JSON")?,
            };
            let services = self.resources.services.as_ref().context("增强服务未绑定")?;
            let response = services.enhance(&self.scope, request).await?;
            Ok(serde_json::to_string(&response)?)
        }
        .await;
        Ok(wire(result))
    }
}

fn wire<T>(result: Result<T>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use database::Host as _;

    #[tokio::test]
    async fn describe_cannot_open_a_database_even_with_a_grant() -> Result<()> {
        let scope = InvocationScope {
            source_id: "source".into(),
            revision: "revision".into(),
            context: Default::default(),
            grants: az_plugin_contract::CapabilityGrants {
                database: true,
                ..Default::default()
            },
        };
        let mut state = InvocationState::new(scope, InvocationResources::default());
        assert!(state.begin().await?.unwrap_err().contains("未授予"));
        state.executing = true;
        state.scope.grants.database = false;
        assert!(state.begin().await?.unwrap_err().contains("未授予"));
        Ok(())
    }
}
