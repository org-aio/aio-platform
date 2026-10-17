use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};

use crate::{
    DatabaseBackend, DatabaseRows, DatabaseTransaction, DatabaseValue, ProcessDatabaseConnection,
};

const DEADLINE: Duration = Duration::from_secs(4);

/// 运行时持有的来源与租户绑定；执行器不接触具体数据库驱动。
#[derive(Clone)]
pub struct ScopedDatabase {
    backend: Arc<dyn DatabaseBackend>,
    source: String,
    tenant: String,
}

impl ScopedDatabase {
    /// Provider 完成迁移和数据库层隔离后，绑定不可变的资源归属。
    pub fn new(backend: Arc<dyn DatabaseBackend>, source: &str, tenant: &str) -> Result<Self> {
        ensure!(
            !source.is_empty() && !tenant.is_empty() && tenant.len() <= 128,
            "数据库归属无效"
        );
        Ok(Self {
            backend,
            source: source.into(),
            tenant: tenant.into(),
        })
    }

    pub(crate) fn matches(&self, source: &str, tenant: &str) -> bool {
        self.source == source && self.tenant == tenant
    }

    #[cfg(feature = "wasm")]
    pub(crate) fn validate_migrations(&self, migrations: &[(String, String)]) -> Result<()> {
        ensure!(migrations.len() <= 64, "迁移数超过配额");
        let mut previous = "";
        for (name, sql) in migrations {
            ensure!(
                name.as_str() > previous && name.len() <= 160 && sql.len() <= 512_000,
                "迁移必须有序且名称唯一"
            );
            previous = name;
        }
        self.backend.validate_migrations(migrations)
    }

    /// 连接只由已绑定后端导出；宿主验证 socket 名称，避免覆盖授权或代理文件。
    pub fn process_connection(&self) -> Result<ProcessDatabaseConnection> {
        let connection = self.backend.process_connection()?;
        let name = &connection.socket_name;
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
            "数据库代理 socket 名称无效"
        );
        Ok(connection)
    }

    /// 开始受统一截止时间约束的事务；取消或丢弃后端句柄必须回滚。
    pub async fn begin(&self) -> Result<ScopedTransaction> {
        let transaction = tokio::time::timeout(DEADLINE, self.backend.begin())
            .await
            .context("开始数据库事务超时")??;
        Ok(ScopedTransaction {
            transaction: Some(transaction),
        })
    }
}

/// 统一配额、超时和失败关闭语义。请求取消时，正在等待的后端事务会被丢弃，
/// 当前句柄保持失效；不能继续使用结果不确定的事务。
pub struct ScopedTransaction {
    transaction: Option<Box<dyn DatabaseTransaction>>,
}

impl ScopedTransaction {
    pub async fn query(
        &mut self,
        statement: &str,
        parameters: Vec<DatabaseValue>,
    ) -> Result<DatabaseRows> {
        let mut transaction = self.transaction.take().context("事务已经结束或失效")?;
        let result = async {
            validate_request(statement, &parameters)?;
            let rows = tokio::time::timeout(DEADLINE, transaction.query(statement, parameters))
                .await
                .context("数据库请求超时")??;
            rows.validate()?;
            Ok(rows)
        }
        .await;
        match result {
            Ok(rows) => {
                self.transaction = Some(transaction);
                Ok(rows)
            }
            Err(error) => Err(rollback_failure(transaction, error).await),
        }
    }

    pub async fn execute(
        &mut self,
        statement: &str,
        parameters: Vec<DatabaseValue>,
    ) -> Result<u64> {
        let mut transaction = self.transaction.take().context("事务已经结束或失效")?;
        let result = async {
            validate_request(statement, &parameters)?;
            tokio::time::timeout(DEADLINE, transaction.execute(statement, parameters))
                .await
                .context("数据库请求超时")?
        }
        .await;
        match result {
            Ok(count) => {
                self.transaction = Some(transaction);
                Ok(count)
            }
            Err(error) => Err(rollback_failure(transaction, error).await),
        }
    }

    pub async fn commit(mut self) -> Result<()> {
        let transaction = self.transaction.take().context("事务已经结束或失效")?;
        tokio::time::timeout(DEADLINE, transaction.commit())
            .await
            .context("提交数据库事务超时")?
    }

    pub async fn rollback(mut self) -> Result<()> {
        if let Some(transaction) = self.transaction.take() {
            tokio::time::timeout(DEADLINE, transaction.rollback())
                .await
                .context("回滚数据库事务超时")??;
        }
        Ok(())
    }
}

fn validate_request(statement: &str, parameters: &[DatabaseValue]) -> Result<()> {
    ensure!(statement.len() <= 64 * 1024, "SQL 语句超过配额");
    ensure!(parameters.len() <= 256, "SQL 参数超过配额");
    Ok(())
}

async fn rollback_failure(
    transaction: Box<dyn DatabaseTransaction>,
    error: anyhow::Error,
) -> anyhow::Error {
    match tokio::time::timeout(DEADLINE, transaction.rollback()).await {
        Ok(Ok(())) => error,
        Ok(Err(rollback)) => error.context(format!("回滚失败: {rollback:#}")),
        Err(_) => error.context("回滚数据库事务超时"),
    }
}
