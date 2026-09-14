use super::*;
use az_plugin_package::PluginPackage;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
};

#[test]
fn published_dependencies_are_downloaded_once_and_reproduce_offline() -> Result<()> {
    let root = tempfile::tempdir()?;
    let manifest = "[plugin.runtime]\nkind='page-definition'\nartifact='dist/pages.json'\n[plugin.frontend]\npath='dist/web'\n[plugin.marketplace]\ntitle='Dependency'\nsummary='Cached dependency'\nlicense='MIT'\ntags=['test']\n";
    let package = PluginPackage::new(
        "https://example.test/dependency.git".into(),
        "1.2.3".into(),
        Some("a".repeat(40)),
        manifest.into(),
        b"[]",
        BTreeMap::from([("index.html".into(), b"<html>Dependency</html>".to_vec())]),
    )?;
    let release = Package::Binary(package.clone()).metadata()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let index = serde_json::to_vec(&vec![release.clone()])?;
    let archive = package.encode()?;
    let server = std::thread::spawn(move || -> Result<()> {
        for response in [index, archive] {
            let (mut stream, _) = listener.accept()?;
            let mut input = BufReader::new(stream.try_clone()?);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                input.read_line(&mut line)?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                request.push_str(&line);
            }
            ensure!(
                request.starts_with("GET /api/runtime/"),
                "意外的发布目录请求"
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )?;
            stream.write_all(&response)?;
        }
        Ok(())
    });
    let mut packages = Packages::new(root.path(), false)?;
    packages.origin = origin;
    packages.client = Client::builder().no_proxy().build()?;
    let candidates = packages.candidates(&release.git, None)?;
    let (path, config) = packages.workspace(&candidates[0])?;
    assert_eq!(std::fs::read(path.join(&config.backend.output))?, b"[]");
    server.join().unwrap()?;
    let lock = LockedPlugin {
        source: release.git.clone(),
        workspace: None,
        source_sha: Some(release.source_sha.clone()),
        content_digest: String::new(),
        package_digest: Some(release.digest.clone()),
        version: Some(release.version.clone()),
        dependencies: vec![],
    };
    let mut offline = Packages::new(root.path(), true)?;
    let cached = offline.candidates(&release.git, Some(&lock))?;
    assert_eq!(cached[0].digest, release.digest);
    assert_eq!(offline.workspace(&cached[0])?.0, path);
    std::fs::write(path.join(&config.backend.output), b"tampered")?;
    assert!(
        offline
            .workspace(&cached[0])
            .unwrap_err()
            .to_string()
            .contains("缓存依赖已被修改")
    );
    Ok(())
}

#[test]
fn component_packages_use_the_same_verified_offline_cache() -> Result<()> {
    let source = tempfile::tempdir()?;
    std::fs::create_dir(source.path().join("web"))?;
    std::fs::write(
        source.path().join("web/index.html"),
        "<html>Component</html>",
    )?;
    std::fs::write(source.path().join("plugin.wasm"), b"\0asm\x0d\0\x01\0")?;
    std::fs::write(
        source.path().join("aio-plugin.toml"),
        "schema_version=2\n[plugin.runtime]\nartifact='plugin.wasm'\nhost_version='>=2026.5.10'\n[plugin.frontend]\npath='web'\n",
    )?;
    let bundle = az_plugin_bundle::Bundle::from_directory(
        source.path(),
        "aio-plugin.toml",
        "https://example.test/component.git".into(),
        "c".repeat(40),
        "2.1.0".into(),
    )?;
    let cache = tempfile::tempdir()?;
    let packages = Packages::new(cache.path(), true)?;
    std::fs::write(
        packages.cache.join(format!("{}.aio-plugin", bundle.digest)),
        bundle.encode()?,
    )?;
    let metadata = Package::Component(bundle).metadata()?;
    let (path, config) = packages.workspace(&metadata)?;
    assert_eq!(config.backend.output, "plugin.wasm");
    assert_eq!(
        std::fs::read(path.join("web/index.html"))?,
        b"<html>Component</html>"
    );
    Ok(())
}
