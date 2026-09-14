use anyhow::{Result, ensure};

const IMAGE: &str = "node@sha256:a81a03dd965b4052269a57fac857004022b522a4bf06e7a739e25e18bce45af2";

pub fn run(args: &[String]) -> Result<()> {
    ensure!(args.len() == 2, "容器适配器需要镜像和产物路径");
    let root = std::env::current_dir()?.join(".aio/dev/container");
    std::fs::create_dir_all(&root)?;
    for (name, content) in [
        ("transport.cjs", include_str!("container/transport.cjs")),
        ("runner.cjs", include_str!("container/runner.cjs")),
        ("worker.cjs", include_str!("container/worker.cjs")),
    ] {
        std::fs::write(root.join(name), content)?;
    }
    let status = std::process::Command::new("node")
        .arg(root.join("runner.cjs"))
        .args(args)
        .arg(IMAGE)
        .status()?;
    ensure!(status.success(), "已发布服务容器退出: {status}");
    Ok(())
}

pub fn cache(args: &[String]) -> Result<()> {
    ensure!(args.len() == 1, "容器缓存需要固定镜像");
    for image in [&args[0], IMAGE] {
        let inspect = std::process::Command::new("docker")
            .args([
                "image",
                "inspect",
                "--format",
                "{{.Os}}/{{.Architecture}}",
                image,
            ])
            .output()?;
        if !inspect.status.success()
            || String::from_utf8_lossy(&inspect.stdout).trim() != "linux/amd64"
        {
            let status = std::process::Command::new("docker")
                .args(["pull", "--platform=linux/amd64", image])
                .status()?;
            ensure!(status.success(), "无法准备容器镜像: {image}");
        }
    }
    std::fs::create_dir_all(".aio/dev")?;
    std::fs::write(".aio/dev/container-images", IMAGE)?;
    Ok(())
}
