use std::sync::Arc;

use anyhow::{Context, Result};
use az_plugin_enhancement::{
    Language, ScriptExecutor, ScriptInvocation, ScriptProgram, ScriptRuntime,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use super::{database, package, scope};
use crate::runtime::server::enhancements::Enhancements;

struct Executor {
    entered: UnboundedSender<()>,
    finished: UnboundedSender<String>,
}
struct Program {
    entered: UnboundedSender<()>,
    finished: UnboundedSender<String>,
}

impl ScriptExecutor for Executor {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, _: &str) -> Result<Arc<dyn ScriptProgram>> {
        Ok(Arc::new(Program {
            entered: self.entered.clone(),
            finished: self.finished.clone(),
        }))
    }
}

impl ScriptProgram for Program {
    fn invoke(&self, input: &Value, invocation: &ScriptInvocation) -> Result<Value> {
        if input == 3 {
            return Ok(json!(6));
        }
        self.entered.send(())?;
        let result = loop {
            if let Err(error) = invocation.check() {
                break error;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        self.finished.send(result.to_string())?;
        Err(result)
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn aborted_callers_cancel_workers_and_restore_execution_capacity() -> Result<()> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(4)
        .max_connections(4)
        .connect(&database()?)
        .await?;
    let (entered, mut entries) = unbounded_channel();
    let (finished, mut exits) = unbounded_channel();
    let runtime = dill::Catalog::builder()
        .add::<ScriptRuntime>()
        .add_value(Executor { entered, finished })
        .bind::<dyn ScriptExecutor, Executor>()
        .build()
        .get_one::<ScriptRuntime>()?;
    let service = Arc::new(Enhancements::open(pool.clone(), runtime, None).await?);
    let tenant = uuid::Uuid::new_v4().to_string();
    let revision = service
        .propose(&tenant, "test", package(Language::Rhai, "fixture"), async {
            Ok(())
        })
        .await?;
    service
        .activate(
            &tenant,
            "test",
            crate::runtime::server::enhancements::Activation {
                revision: Some(revision.clone()),
                expected_generation: 0,
            },
            false,
            async { Ok(()) },
        )
        .await?
        .context("未激活")?;
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let service = service.clone();
        let scope = scope(&tenant);
        tasks.push(tokio::spawn(async move {
            service.invoke(&scope, super::request(), None).await
        }));
    }
    for _ in 0..4 {
        tokio::time::timeout(std::time::Duration::from_secs(2), entries.recv())
            .await?
            .context("工作线程未进入执行")?;
    }
    for task in tasks {
        task.abort();
        match task.await {
            Err(error) => assert!(error.is_cancelled(), "{error}"),
            Ok(result) => anyhow::bail!("请求在取消前已结束: {result:?}"),
        }
    }
    for _ in 0..4 {
        let error = tokio::time::timeout(std::time::Duration::from_secs(2), exits.recv())
            .await?
            .context("工作线程未退出")?;
        assert!(error.contains("已取消"), "{error}");
    }
    // 工作线程发出退出信号后仍需完成栈释放；等待一个执行槽确实归还后再验收新调用。
    let request = az_plugin_contract::EnhancementRequest {
        path: "/v1/math/double".into(),
        input: json!(3),
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match service.invoke(&scope(&tenant), request.clone(), None).await {
                Ok(response) => break Ok::<_, anyhow::Error>(response),
                Err(error) if error.to_string().contains("并发已满") => {
                    tokio::task::yield_now().await
                }
                Err(error) => break Err(error),
            }
        }
    })
    .await??;
    assert_eq!(result.output, json!(6));
    pool.close().await;
    Ok(())
}
