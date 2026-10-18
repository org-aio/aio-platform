//! `aio-launcher` 可执行入口。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    az_plugin_launcher::run_cli().await
}
