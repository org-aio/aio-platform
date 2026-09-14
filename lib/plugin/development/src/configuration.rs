use crate::DevConfiguration;
use anyhow::{Context, Result, ensure};
use std::path::{Component, Path};

pub fn read(root: &Path) -> Result<DevConfiguration> {
    let path = root.join("aio-dev.toml");
    let config: DevConfiguration = toml::from_str(
        &std::fs::read_to_string(&path)
            .with_context(|| format!("读取开发配置失败: {}", path.display()))?,
    )?;
    ensure!(config.version == 1, "不支持的 aio-dev.toml 版本");
    for task in [&config.frontend, &config.backend]
        .into_iter()
        .chain(config.prepare.iter())
    {
        ensure!(!task.command.is_empty(), "开发构建命令不能为空");
        ensure!(!task.inputs.is_empty(), "开发任务需要声明输入");
        validate_relative(&task.output)?;
        for input in &task.inputs {
            validate_relative(input)?;
        }
    }
    if let Some(run) = &config.run {
        ensure!(!run.command.is_empty(), "启动命令不能为空");
        ensure!(
            run.health.starts_with('/') && !run.health.starts_with("//"),
            "健康检查路径无效"
        );
    }
    Ok(config)
}

pub fn validate_relative(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && Path::new(value)
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
        "路径必须位于插件目录内: {value}"
    );
    Ok(())
}
