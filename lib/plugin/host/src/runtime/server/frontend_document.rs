use anyhow::{Context as _, Result, ensure};
use kuchikiki::traits::TendrilSink as _;

pub(super) const BRIDGE_PATH: &str = "__aio_bridge.js";
pub(super) const MODULES_PATH: &str = "__aio_modules.js";
pub(super) const MODULES: &[u8] = include_bytes!("vendor/es-module-shims.js");

pub(super) fn render_entry(
    html: &[u8],
    prefix: &str,
    entry: &str,
    token: &str,
    development: bool,
) -> Result<Vec<u8>> {
    let html = std::str::from_utf8(html).context("前端入口不是 UTF-8")?;
    let document = kuchikiki::parse_html().one(html).document_node;
    let head = document
        .select_first("head")
        .map_err(|_| anyhow::anyhow!("前端入口缺少 head"))?;
    for script in document
        .select("script[type='module'], script[type='importmap']")
        .map_err(|_| anyhow::anyhow!("解析模块入口失败"))?
    {
        let mut attributes = script.attributes.borrow_mut();
        let kind = if attributes.get("type") == Some("module") {
            "module-shim"
        } else {
            "importmap-shim"
        };
        attributes.insert("type", kind.to_owned());
    }
    for node in document
        .select("base")
        .map_err(|_| anyhow::anyhow!("解析入口 base 失败"))?
    {
        node.as_node().detach();
    }
    // 浏览器解析器构造节点，避免对插件 HTML 做字符串替换。
    let bootstrap = kuchikiki::parse_html()
        .one("<head><base><script></script></head>")
        .document_node;
    let base = bootstrap
        .select_first("base")
        .map_err(|_| anyhow::anyhow!("创建前端 base 失败"))?;
    let directory = entry
        .rsplit_once('/')
        .map(|(directory, _)| format!("{directory}/"))
        .unwrap_or_default();
    base.attributes
        .borrow_mut()
        .insert("href", format!("{prefix}{directory}"));
    let script = bootstrap
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建通信桥失败"))?;
    script
        .attributes
        .borrow_mut()
        .insert("src", format!("{prefix}{BRIDGE_PATH}"));
    script
        .attributes
        .borrow_mut()
        .insert("data-token", token.to_owned());
    script
        .attributes
        .borrow_mut()
        .insert("data-development", development.to_string());
    let script_node = script.as_node().clone();
    script_node.detach();
    let loader = kuchikiki::parse_html()
        .one("<head><script></script></head>")
        .document_node;
    let loader = loader
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建模块加载器失败"))?;
    loader
        .attributes
        .borrow_mut()
        .insert("src", format!("{prefix}{MODULES_PATH}"));
    let loader = loader.as_node().clone();
    loader.detach();
    head.as_node().prepend(loader);
    head.as_node().prepend(script_node);
    let base_node = base.as_node().clone();
    base_node.detach();
    head.as_node().prepend(base_node);
    let mut output = Vec::new();
    document
        .serialize(&mut output)
        .context("序列化前端入口失败")?;
    ensure!(
        output.len() <= az_plugin_package::MAX_ARTIFACT_BYTES,
        "前端入口过大"
    );
    Ok(output)
}

pub(super) fn content_policy(prefix: &str) -> String {
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob: {prefix}; connect-src blob: {prefix}; style-src 'unsafe-inline' blob: {prefix}; img-src data: blob: {prefix}; font-src data: {prefix}; frame-src 'none'; object-src 'none'; worker-src 'none'; form-action 'none'; base-uri {prefix}; frame-ancestors 'self'"
    )
}

pub(super) fn public_origin(configured: &str) -> Result<String> {
    let url = reqwest::Url::parse(configured).context("AIO_PUBLIC_ORIGIN 无效")?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && url.host_str().is_some_and(|host| host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|address| address.is_loopback()))),
        "前端公开地址必须为 HTTPS 或本机 HTTP"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "AIO_PUBLIC_ORIGIN 只能包含协议与主机"
    );
    Ok(url.origin().ascii_serialization())
}

pub(super) static BRIDGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{}\n{}",
        az_plugin_runtime::FRONTEND_LIFECYCLE,
        include_str!("frontend_guest.js")
    )
});
