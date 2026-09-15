mod execution;
mod storage;

use crate::{InstallLink, ToolManifest};
use anyhow::{Context as _, Result, ensure};
use std::io::{self, IsTerminal, Read, Write};
pub(crate) use storage::helper_root;
pub use storage::{Installation, Store};

pub fn fetch(link: &InstallLink) -> Result<ToolManifest> {
    // 协议调用不能更换市场来源，也不接受重定向到其他下载站。
    let url = format!(
        "{}/api/runtime/tools/{}/{}",
        crate::OFFICIAL_ORIGIN,
        link.id,
        link.version
    );
    fetch_from(&url, link)
}

pub(crate) fn fetch_from(url: &str, link: &InstallLink) -> Result<ToolManifest> {
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(url)
        .send()?
        .error_for_status()?;
    ensure!(response.status().is_success(), "市场返回了重定向或无效状态");
    let mut bytes = Vec::new();
    response
        .take(crate::MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= crate::MAX_MANIFEST_BYTES,
        "安装描述超过大小限制"
    );
    let manifest: ToolManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    ensure!(
        manifest.id == link.id && manifest.version == link.version,
        "市场返回的工具或版本与请求不符"
    );
    Ok(manifest)
}

pub fn confirm(manifest: &ToolManifest, uninstall: bool) -> Result<bool> {
    let plan = manifest
        .platforms
        .get(std::env::consts::OS)
        .context("此工具不支持当前系统")?;
    println!(
        "{} {} · {}",
        manifest.title, manifest.version, manifest.homepage
    );
    println!("目标：此电脑的当前用户。{}", manifest.summary);
    if !uninstall {
        println!("依赖检测与安装后验证：");
        for command in plan
            .requirements
            .iter()
            .map(|r| &r.check)
            .chain([&plan.detect])
        {
            println!(
                "  {} {}",
                command.program,
                serde_json::to_string(&command.args)?
            );
        }
        println!("安装步骤：");
    }
    for command in if uninstall {
        &plan.uninstall
    } else {
        &plan.install
    } {
        println!(
            "  {} {}",
            command.program,
            serde_json::to_string(&command.args)?
        );
    }
    ensure!(io::stdin().is_terminal(), "请在终端运行此操作并确认");
    print!(
        "确认{}？输入 yes：",
        if uninstall {
            "恢复配置并卸载"
        } else {
            "安装"
        }
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == "yes")
}

impl Store {
    pub fn install(&self, manifest: ToolManifest) -> Result<()> {
        manifest.validate()?;
        let _lock = self.lock()?;
        let plan = manifest
            .platforms
            .get(std::env::consts::OS)
            .context("此工具不支持当前系统")?;
        if let Some(previous) = self.read(&manifest.id)? {
            ensure!(
                previous.manifest == manifest,
                "已有其他版本或未完成安装，请先执行 aio tool uninstall {} 恢复配置",
                manifest.id
            );
        }
        execution::requirements(&plan.requirements)?;
        let mut record = Installation {
            manifest: manifest.clone(),
            state: "installing".into(),
            completed_steps: 0,
        };
        self.save(&record)?;
        for command in &plan.install {
            if let Err(error) = execution::run(command) {
                record.state = "failed".into();
                self.save(&record)?;
                return Err(error.context(format!(
                    "安装未完成；运行 aio tool uninstall {} 可恢复配置",
                    manifest.id
                )));
            }
            record.completed_steps += 1;
            self.save(&record)?;
        }
        if let Err(error) = execution::run(&plan.detect) {
            record.state = "failed".into();
            self.save(&record)?;
            return Err(error.context("安装命令已结束，但检测未通过"));
        }
        record.state = "installed".into();
        self.save(&record)?;
        println!("已安装并验证 {} {}", manifest.id, manifest.version);
        Ok(())
    }

    pub fn uninstall(&self, id: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut record = self.read(id)?.context("未找到该工具的安装记录")?;
        let commands = record
            .manifest
            .platforms
            .get(std::env::consts::OS)
            .context("安装记录的平台不匹配")?
            .uninstall
            .clone();
        // 重试卸载时从上次完成的位置继续，防止重复恢复已经撤销的配置。
        if record.state != "uninstalling" {
            record.completed_steps = 0;
        }
        record.state = "uninstalling".into();
        self.save(&record)?;
        for command in commands.iter().skip(record.completed_steps) {
            execution::run(command).context("卸载未完成，已保留记录；修复错误后可重试")?;
            record.completed_steps += 1;
            self.save(&record)?;
        }
        self.remove(id)?;
        println!("已恢复配置并卸载 {id}");
        Ok(())
    }
}
