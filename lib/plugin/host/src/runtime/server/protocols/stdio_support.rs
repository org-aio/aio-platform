use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

pub(super) const LITERAL: &str = "$(touch forbidden); `touch forbidden` $HOME \"literal\"";

pub(super) struct Upstream {
    pub root: tempfile::TempDir,
    program: PathBuf,
    factor: i64,
}

impl Upstream {
    pub fn new(factor: i64) -> Result<Self> {
        ensure!(
            std::env::var("AIO_MCP_STDIO_TEST_TOKEN")? == "stdio-test",
            "只接受测试凭据"
        );
        ensure!(
            std::env::var_os("AIO_MCP_STDIO_HOST_SECRET").is_some(),
            "需要测试宿主环境哨兵"
        );
        let program = PathBuf::from(
            std::env::var("AIO_MCP_STDIO_TEST_EXECUTABLE")
                .context("先构建 mcp-stdio-fixture 并配置测试路径")?,
        );
        ensure!(
            program.is_absolute() && program.is_file(),
            "测试程序必须存在且使用绝对路径"
        );
        let root = tempfile::tempdir()?;
        let fixture = Self {
            root,
            program,
            factor,
        };
        fixture.mode("normal")?;
        Ok(fixture)
    }

    pub fn mode(&self, mode: &str) -> Result<()> {
        std::fs::write(self.root.path().join("mode"), mode)?;
        Ok(())
    }

    pub fn import(&self) -> Value {
        json!({"tenant_id":"test","capability":{"path":"/v1/remote/mcp","description":"外部计算器","effect":"read","input_schema":super::super::upstream::schema(),"output_schema":super::super::upstream::schema()},"transport":{"protocol":"mcp_stdio","tool":"double","output":"structured","command":{"program":self.program,"args":[self.root.path(),self.factor.to_string(),LITERAL],"cwd":self.root.path(),"env":{"CHILD_TOKEN":"AIO_MCP_STDIO_TEST_TOKEN"}}}})
    }

    pub fn records(&self) -> Result<Vec<Value>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(self.root.path())? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                records.push(serde_json::from_slice(&std::fs::read(path)?)?);
            }
        }
        Ok(records)
    }

    pub fn count(&self, extension: &str) -> Result<usize> {
        let mut count = 0;
        for entry in std::fs::read_dir(self.root.path())? {
            if entry?
                .path()
                .extension()
                .is_some_and(|ext| ext == extension)
            {
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn event(&self, extension: &str, previous: usize) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.count(extension)? <= previous {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("子进程没有到达测试阶段")??;
        Ok(())
    }

    pub async fn reaped(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut active = false;
                for record in self.records()? {
                    let parent = record["parent"].as_u64().context("缺少父进程")?;
                    let descendant = record["descendant"].as_u64().context("缺少后代进程")?;
                    // 直接子进程必须被 wait；非本进程的后代退出后由其系统收养者回收。
                    active |=
                        PathBuf::from(format!("/proc/{parent}")).exists() || running(descendant)?;
                    ensure!(
                        record["env_ok"] == true
                            && record["cwd_ok"] == true
                            && record["literal"] == LITERAL,
                        "子进程环境、目录或字面参数不符合约定"
                    );
                }
                if !active {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("MCP stdio 留下了未回收的子进程或运行中的后代")??;
        ensure!(
            !self.root.path().join("forbidden").exists(),
            "参数被当作 shell 执行"
        );
        Ok(())
    }
}

fn running(pid: u64) -> Result<bool> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let state = stat
        .rsplit_once(") ")
        .context("无效进程状态")?
        .1
        .chars()
        .next();
    Ok(!matches!(state, Some('Z' | 'X')))
}

impl Drop for Upstream {
    fn drop(&mut self) {
        // 测试失败也只清理此夹具仍在运行的进程组，不操作其他宿主进程。
        if let Ok(records) = self.records() {
            for record in records {
                let Some(parent) = record["parent"].as_u64() else {
                    continue;
                };
                let Some(descendant) = record["descendant"].as_u64() else {
                    continue;
                };
                if running(parent).unwrap_or(false) || running(descendant).unwrap_or(false) {
                    let result = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(parent as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                    if let Err(error) = result {
                        eprintln!("清理测试进程失败: {error}");
                    }
                }
            }
        }
    }
}
