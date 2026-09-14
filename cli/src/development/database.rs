use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio};
use tokio::process::Command;

pub(super) struct Database {
    pub url: String,
    container: Option<String>,
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Some(name) = &self.container {
            let _ = std::process::Command::new("docker")
                .args(["stop", "--time", "5", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

pub(super) async fn prepare(root: &Path, explicit: Option<&str>) -> Result<Database> {
    if let Some(url) = explicit {
        return Ok(Database {
            url: url.into(),
            container: None,
        });
    }
    let saved = root.join("database-connection.json");
    if saved.is_file() {
        let url: String = serde_json::from_slice(&std::fs::read(saved)?)?;
        return Ok(Database {
            url,
            container: None,
        });
    }
    let path = root.join("database.json");
    let (name, password) = if path.is_file() {
        let value: (String, String) = serde_json::from_slice(&std::fs::read(&path)?)?;
        value
    } else {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let value = (
            format!("aio-dev-{}", &id[..12]),
            uuid::Uuid::new_v4().simple().to_string(),
        );
        super::process::private_file(&path, &serde_json::to_vec(&value)?)?;
        value
    };
    let exists = Command::new("docker")
        .kill_on_drop(true)
        .args(["inspect", &name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("本地沙箱需要 Docker，或通过 --database-url 指定独立开发 PostgreSQL")?
        .success();
    let database = Database {
        url: String::new(),
        container: Some(name.clone()),
    };
    let mut command = Command::new("docker");
    command.kill_on_drop(true);
    if exists {
        command.args(["start", &name]);
    } else {
        command.args([
            "run",
            "-d",
            "--name",
            &name,
            "--label",
            "site.addzero.aio.development=true",
            "-p",
            "127.0.0.1::5432",
            "-e",
            "POSTGRES_DB=aio_development",
            "-e",
            "POSTGRES_USER=developer",
            "-e",
            "POSTGRES_PASSWORD",
            "-v",
            &format!("{name}:/var/lib/postgresql/data"),
            "postgres:17.6-bookworm",
        ]);
        command.env("POSTGRES_PASSWORD", &password);
    }
    ensure!(
        command.stdout(Stdio::null()).status().await?.success(),
        "启动开发 PostgreSQL 失败"
    );
    let output = Command::new("docker")
        .kill_on_drop(true)
        .args(["port", &name, "5432/tcp"])
        .output()
        .await?;
    ensure!(output.status.success(), "读取开发数据库端口失败");
    let address = String::from_utf8(output.stdout)?.trim().to_string();
    let mut database = database;
    database.url = format!("postgres://developer:{password}@{address}/aio_development");
    for _ in 0..60 {
        if Command::new("docker")
            .kill_on_drop(true)
            .args([
                "exec",
                &name,
                "pg_isready",
                "-U",
                "developer",
                "-d",
                "aio_development",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?
            .success()
        {
            return Ok(database);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    anyhow::bail!("开发 PostgreSQL 60 秒内未就绪")
}
