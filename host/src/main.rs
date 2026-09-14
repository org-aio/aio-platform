#[cfg(feature = "server")]
mod server;

#[cfg(all(feature = "server", feature = "web"))]
compile_error!("分别构建宿主服务与 Web 资源");

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::run().await
}

#[cfg(feature = "web")]
fn main() {
    dioxus::launch(App);
}

#[cfg(feature = "web")]
#[allow(non_snake_case)]
fn App() -> dioxus::prelude::Element {
    use dioxus::prelude::*;
    rsx! {
        az_ui_components::UiStylesheets {}
        az_plugin_host::DevelopmentStatus {}
        az_plugin_host::Workspace {
            config: az_plugin_host::composition::BrowserComposition {
                label: "AIO Sandbox".into(), pages: vec![], account_items: vec![],
                login: || rsx! { p { "开发会话已结束" } }, account_action: |_| {},
            }
        }
    }
}
