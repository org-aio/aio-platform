mod build;
mod documents;
mod packaging;
mod toolchains;

use std::{
    collections::HashSet,
    env,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use az_plugin_delivery::{BuildJob, BuildReport};
use reqwest::blocking::Client;

pub struct Worker {
    client: Client,
    base: String,
    token: String,
    root: PathBuf,
}

impl Worker {
    pub fn request(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    fn run(&self, job: &BuildJob) -> Result<()> {
        let root = self.root.join(format!("job-{}", job.id));
        let result = build::execute(self, job, &root);
        if let Err(error) = &result {
            let heartbeat = self
                .request(&format!("/api/internal/delivery/jobs/{}/heartbeat", job.id))
                .timeout(Duration::from_secs(10))
                .json(&serde_json::json!({"lease":job.lease}))
                .send();
            if heartbeat.is_ok_and(|response| response.status().as_u16() == 409) {
                std::fs::remove_dir_all(&root)?;
                return Ok(());
            }
            if error.downcast_ref::<build::Retryable>().is_some()
                || error.chain().any(|cause| {
                    cause.downcast_ref::<reqwest::Error>().is_some_and(|e| {
                        e.is_timeout()
                            || e.is_connect()
                            || e.status().is_some_and(|status| {
                                status.is_server_error() || status.as_u16() == 429
                            })
                    })
                })
            {
                self.request(&format!("/api/internal/delivery/jobs/{}/defer", job.id))
                    .timeout(Duration::from_secs(10))
                    .json(&serde_json::json!({"lease": job.lease, "error": format!("{error:#}")}))
                    .send()?
                    .error_for_status()?;
                return Err(anyhow::anyhow!("网络错误，保留产物等待租约重试: {error:#}"));
            }
        }
        let report = match result {
            Ok(documentation) => BuildReport {
                lease: job.lease.clone(),
                error: None,
                documentation,
            },
            Err(error) => BuildReport {
                lease: job.lease.clone(),
                error: Some(format!("{error:#}")),
                documentation: Default::default(),
            },
        };
        self.request(&format!("/api/internal/delivery/jobs/{}/complete", job.id))
            .json(&report)
            .send()?
            .error_for_status()?;
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        Ok(())
    }

    fn cleanup(&self) -> Result<()> {
        let retained: HashSet<i64> = self
            .request("/api/internal/delivery/retained-jobs")
            .timeout(Duration::from_secs(10))
            .send()?
            .error_for_status()?
            .json()?;
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_prefix("job-"))
                .and_then(|id| id.parse::<i64>().ok())
            else {
                continue;
            };
            if !retained.contains(&id) && entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let worker = Worker {
        client: Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()?,
        base: env::var("AIO_DELIVERY_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3080".into())
            .trim_end_matches('/')
            .into(),
        token: env::var("AIO_DELIVERY_TOKEN").context("缺少 AIO_DELIVERY_TOKEN")?,
        root: env::var_os("AIO_DELIVERY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/opt/aio-delivery".into()),
    };
    std::fs::create_dir_all(&worker.root)?;
    build::recover_containers()?;
    let mut delay = 10;
    let mut cleaned = Instant::now();
    loop {
        if cleaned.elapsed() >= Duration::from_secs(300) {
            if let Err(error) = worker.cleanup() {
                eprintln!("清理交付临时产物失败: {error:#}");
            }
            cleaned = Instant::now();
        }
        let result = (|| -> Result<()> {
            let job: Option<BuildJob> = worker
                .request("/api/internal/delivery/claim")
                .send()?
                .error_for_status()?
                .json()?;
            if let Some(job) = job {
                worker.run(&job)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("交付工作进程: {error:#}");
            delay = (delay * 2).min(300);
        } else {
            delay = 10;
        }
        thread::sleep(Duration::from_secs(delay));
    }
}
