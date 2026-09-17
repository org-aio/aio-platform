pub const WORKBENCH_GIT: &str = "https://github.com/zjarlin/dioxus-admin-workbench.git";
pub const WORKBENCH_REV: &str = "819ae159ab7dcba65d33d056d528ae8f7ec32ae2";

pub const GITIGNORE: &str = "/target\n/.aio/plugins\n";

pub const RUST_TOOLCHAIN: &str = r#"[toolchain]
channel = "nightly"
components = ["rustfmt", "clippy"]
profile = "minimal"
"#;

pub const INDEX_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
    <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <title>{app_title}</title>
        <link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='6' fill='%23111827'/%3E%3Cpath d='M8 24L15 7h2l7 17h-4l-1.5-4h-7L10 24zM13 16h4l-2-5z' fill='white'/%3E%3C/svg%3E">
    </head>
    <body>
        <div id="main"></div>
    </body>
</html>
"#;

pub fn application_cargo(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1.0.99"
axum = {{ version = "0.8.6", optional = true }}
dill = "0.15.0"
dioxus = {{ version = "=0.7.9", default-features = false, features = ["macro", "html", "signals"], optional = true }}
az-dioxus-admin-shell = {{ git = "{WORKBENCH_GIT}", rev = "{WORKBENCH_REV}", default-features = false, optional = true }}
tokio = {{ version = "1.47.1", features = ["macros", "net", "rt-multi-thread"], optional = true }}
tower-http = {{ version = "0.6.2", features = ["fs"], optional = true }}

[features]
default = ["web"]
ui = ["dep:dioxus", "dep:az-dioxus-admin-shell"]
web = ["ui", "dioxus/web", "dioxus/launch"]
desktop = ["ui", "dioxus/desktop", "dioxus/launch"]
server = ["dep:axum", "dep:tokio", "dep:tower-http"]

[lints.rust]
unsafe_code = "forbid"
"#
    )
}

pub fn dioxus_manifest(name: &str, title: &str) -> String {
    format!(
        "[application]\nname = {}\ndefault_platform = \"web\"\n\n[web.app]\ntitle = {}\n",
        toml_string(name),
        toml_string(title)
    )
}

pub fn aio_manifest(name: &str, title: &str) -> String {
    format!(
        "plugins = []\n\n[application]\nname = {}\ntitle = {}\n",
        toml_string(name),
        toml_string(title)
    )
}

pub fn application_readme(title: &str) -> String {
    format!(
        "# {title}\n\n```bash\ndx serve\ncargo run --no-default-features --features desktop\ncargo run --no-default-features --features server\naio plugin install <git>\naio plugin sync\n```\n"
    )
}

pub fn application_main(title: &str) -> String {
    let source = r#"#![forbid(unsafe_code)]

mod plugins;
#[cfg(feature = "server")]
mod server;

#[cfg(all(feature = "server", any(feature = "web", feature = "desktop")))]
compile_error!("server 不能和 web 或 desktop 同时启用");

#[cfg(not(any(feature = "web", feature = "desktop", feature = "server")))]
fn main() {
    eprintln!("请选择 --features web、desktop 或 server");
}

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::run().await
}

#[cfg(any(feature = "web", feature = "desktop"))]
fn main() {
    dioxus::launch(App);
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[allow(non_snake_case)]
fn App() -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{ApplicationUser, PluginApplication};
    use dioxus::prelude::*;

    let catalog = match plugins::client_catalog() {
        Ok(catalog) => catalog,
        Err(error) => return rsx! { p { "加载应用页面失败: {error}" } },
    };
    rsx! {
        PluginApplication {
            application_label: __APPLICATION_TITLE__,
            pages: catalog.pages,
            topbar_items: catalog.topbar_items,
            user: ApplicationUser {
                label: "用户".to_owned(),
                handle: String::new(),
                initials: "U".to_owned(),
            },
        }
    }
}
"#;
    source.replace("__APPLICATION_TITLE__", &rust_string(title))
}

pub fn application_server(name: &str) -> String {
    let source = r#"use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context as _, Result};
use axum::{Router, routing::get};
use tower_http::services::{ServeDir, ServeFile};

use crate::plugins;

pub async fn run() -> Result<()> {
    let port = env::var("AIO_WEB_PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse::<u16>()
        .context("AIO_WEB_PORT 必须是有效端口")?;
    let web_dist = env::var_os("AIO_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dx/__APPLICATION_NAME__/release/web/public"));
    let index = web_dist.join("index.html");
    let application = ServeDir::new(web_dist).fallback(ServeFile::new(index));
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(plugins::server_router()?)
        .fallback_service(application);
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("绑定监听地址失败: {address}"))?;
    println!("AIO listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router).await.context("AIO 服务异常退出")
}
"#;
    source.replace("__APPLICATION_NAME__", name)
}

pub fn plugins_source(clients: &[String], servers: &[String]) -> String {
    let client_registrations = clients
        .iter()
        .map(|dependency| format!("    {dependency}::register(&mut builder);\n"))
        .collect::<String>();
    let server_registrations = servers
        .iter()
        .map(|dependency| format!("    {dependency}::register(&mut builder)?;\n"))
        .collect::<String>();
    let server_routers = servers
        .iter()
        .map(|dependency| {
            format!(
                "    router = router.merge({dependency}::router(&catalog).context(\"装配服务端插件失败\")?);\n"
            )
        })
        .collect::<String>();
    let source = r#"// 此文件由 aio plugin sync 生成。
#[cfg(any(feature = "web", feature = "desktop"))]
pub struct ClientCatalog {
    pub pages: Vec<az_dioxus_admin_shell::ApplicationPage>,
    pub topbar_items: Vec<az_dioxus_admin_shell::ApplicationTopbarItem>,
}

#[cfg(any(feature = "web", feature = "desktop"))]
pub fn client_catalog() -> anyhow::Result<ClientCatalog> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
__CLIENT_REGISTRATIONS__    builder.validate().context("校验页面插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    Ok(ClientCatalog {
        pages: az_dioxus_admin_shell::collect_application_pages(&catalog)?,
        topbar_items: az_dioxus_admin_shell::collect_application_topbar_items(&catalog)?,
    })
}

#[cfg(feature = "server")]
pub fn server_router() -> anyhow::Result<axum::Router> {
    use anyhow::Context as _;
    use dill::{Catalog, CatalogBuilder};

    let mut builder = CatalogBuilder::new();
__SERVER_REGISTRATIONS__    builder.validate().context("校验服务端插件依赖图失败")?;
    let catalog: Catalog = builder.build();
    let mut router = axum::Router::new();
__SERVER_ROUTERS__    Ok(router)
}
"#;
    source
        .replace("__CLIENT_REGISTRATIONS__", &client_registrations)
        .replace("__SERVER_REGISTRATIONS__", &server_registrations)
        .replace("__SERVER_ROUTERS__", &server_routers)
}

pub fn repository_workspace(client_name: &str, server_name: &str) -> String {
    format!(
        "[workspace]\nresolver = \"2\"\nmembers = [\"client\", \"server\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n\n# Cargo 包：{client_name}、{server_name}\n"
    )
}

pub fn repository_client_cargo(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version.workspace = true
edition.workspace = true

[dependencies]
dill = "0.15.0"
dioxus = {{ version = "=0.7.9", default-features = false, features = ["macro", "html"] }}
az-dioxus-admin-shell = {{ git = "{WORKBENCH_GIT}", rev = "{WORKBENCH_REV}", default-features = false }}

[lints]
workspace = true
"#
    )
}

pub fn repository_server_cargo(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version.workspace = true
edition.workspace = true

[dependencies]
anyhow = "1.0.99"
axum = "0.8.6"
dill = "0.15.0"

[lints]
workspace = true
"#
    )
}

pub fn repository_plugin_readme(title: &str) -> String {
    format!(
        "# {title}\n\n此仓库是 AIO 全栈插件。`client` 贡献页面，`server` 贡献 HTTP 能力，两端都通过 Dill 注册具体类型。提交到 Git 后执行：\n\n```bash\naio plugin install <git>\n```\n"
    )
}

pub fn repository_client_readme() -> &'static str {
    "# 页面能力\n\n此 crate 只负责向 AIO 壳注册页面，不拥有服务端职责。\n"
}

pub fn repository_server_readme() -> &'static str {
    "# 服务能力\n\n此 crate 只负责注册服务并装配 HTTP 路由，不拥有前端职责。`register` 返回 `anyhow::Result<()>`，宿主会传播装配失败。\n"
}

pub fn repository_client_source(title: &str) -> String {
    let source = r#"#![forbid(unsafe_code)]

use az_dioxus_admin_shell::{ApplicationPage, ApplicationPlugin, ApplicationScene};
use dill::CatalogBuilder;
use dioxus::prelude::*;

#[derive(Debug)]
struct PagesPlugin;

impl ApplicationPlugin for PagesPlugin {
    fn pages(&self) -> Vec<ApplicationPage> {
        vec![ApplicationPage {
            id: "plugin-page",
            label: __PAGE_TITLE__,
            icon: Some("panels-top-left"),
            scene: ApplicationScene {
                id: "workspace",
                label: "工作区",
            },
            menu_path: Vec::new(),
            required_permission: None,
            render: PluginPage,
        }]
    }
}

pub fn register(builder: &mut CatalogBuilder) {
    builder
        .add_value(PagesPlugin)
        .bind::<dyn ApplicationPlugin, PagesPlugin>();
}

#[allow(non_snake_case)]
fn PluginPage() -> Element {
    rsx! {
        section {
            h2 { __PAGE_TITLE__ }
        }
    }
}
"#;
    source.replace("__PAGE_TITLE__", &rust_string(title))
}

pub fn repository_server_source(route_slug: &str) -> String {
    let source = r#"#![forbid(unsafe_code)]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::{Router, routing::get};
use dill::{Catalog, CatalogBuilder};

#[derive(Debug)]
struct StatusService;

impl StatusService {
    fn health(&self) -> &'static str {
        "ok"
    }
}

pub fn register(builder: &mut CatalogBuilder) -> Result<()> {
    builder.add_value(StatusService);
    Ok(())
}

pub fn router(catalog: &Catalog) -> Result<Router> {
    let service = catalog
        .get_one::<StatusService>()
        .context("解析插件状态服务失败")?;
    Ok(Router::new().route(
        "/api/plugins/__ROUTE_SLUG__/health",
        get(move || {
            let service = Arc::clone(&service);
            async move { service.health() }
        }),
    ))
}
"#;
    source.replace("__ROUTE_SLUG__", route_slug)
}

pub fn repository_manifest() -> &'static str {
    "[plugin.client]\npath = \"client\"\n\n[plugin.server]\npath = \"server\"\n"
}

pub fn dockerfile(name: &str) -> String {
    format!(
        r#"FROM rustlang/rust:nightly-bookworm AS build
RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked
WORKDIR /source
COPY . .
RUN dx build --platform web --release \
    && cargo build --release --no-default-features --features server

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 aio
WORKDIR /opt/aio
COPY --from=build /source/target/release/{name} /usr/local/bin/aio-application
COPY --from=build /source/target/dx/{name}/release/web/public /opt/aio/web
ENV AIO_WEB_PORT=8080
ENV AIO_WEB_DIST=/opt/aio/web
EXPOSE 8080
USER aio
ENTRYPOINT ["/usr/local/bin/aio-application"]
"#
    )
}

pub const COMPOSE_MANIFEST: &str = r#"services:
  aio:
    build: .
    restart: unless-stopped
    ports:
      - "127.0.0.1:3080:8080"
"#;

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}
