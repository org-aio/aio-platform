use std::{
    io::Write as _,
    net::{TcpListener, TcpStream},
};

use anyhow::{Result, anyhow};
use az_plugin_bundle::Bundle;
use az_plugin_package::PluginPackage;

use super::*;

#[test]
fn posts_raw_binary_package_and_polls_activation() -> Result<()> {
    let expected = package()?;
    let package = PreparedRelease::Legacy(expected.clone());
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!(
        "http://{}/api/runtime/plugins/publish",
        listener.local_addr()?
    );
    let server_expected = expected.clone();
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        let (headers, body) = read_request(&mut stream)?;
        assert!(headers.starts_with("POST /api/runtime/plugins/publish HTTP/1.1\r\n"));
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("content-type: application/vnd.aio.plugin+gzip\r\n"));
        assert!(!headers.contains("content-encoding:"));
        assert!(headers.contains("authorization: bearer test-token\r\n"));
        assert_eq!(PluginPackage::decode(&body)?, server_expected);
        respond(&mut stream, &server_expected.rev, "queued")?;
        let (mut stream, _) = listener.accept()?;
        let (headers, body) = read_request(&mut stream)?;
        assert!(headers.starts_with("GET /api/runtime/publish-jobs/job-1 HTTP/1.1\r\n"));
        assert!(body.is_empty());
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token\r\n")
        );
        respond(&mut stream, &server_expected.rev, "active")
    });
    let active = publish_to(&package, &endpoint, "test-token")?;
    assert_eq!(active.revision, expected.rev);
    assert_eq!(active.page_count, 1);
    server.join().map_err(|_| anyhow!("发布测试线程失败"))??;
    Ok(())
}

#[test]
fn rejects_wrong_revision_and_reports_activation_failure() -> Result<()> {
    for (revision, state, expected_error) in [
        ("0".repeat(64), "active", "不一致的内容版本"),
        (package()?.rev, "failed", "插件发布失败"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let endpoint = format!(
            "http://{}/api/runtime/plugins/publish",
            listener.local_addr()?
        );
        let server = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            respond(&mut stream, &revision, state)
        });
        let package = PreparedRelease::Legacy(package()?);
        let error = publish_to(&package, &endpoint, "test-token")
            .err()
            .context("无效响应必须失败")?;
        assert!(error.to_string().contains(expected_error));
        server.join().map_err(|_| anyhow!("发布测试线程失败"))??;
    }
    Ok(())
}

#[test]
fn accepts_only_https_or_loopback_endpoints_and_safe_job_paths() -> Result<()> {
    let package = PreparedRelease::Legacy(package()?);
    assert!(publish_url("http://example.com/api/runtime/plugins/publish", &package).is_err());
    assert!(publish_url("https://example.com/other", &package).is_err());
    assert!(
        publish_url(
            "https://a:b@example.com/api/runtime/plugins/publish",
            &package
        )
        .is_err()
    );
    assert!(publish_url("http://[::1]:8080/api/runtime/plugins/publish", &package).is_ok());
    assert!(
        publish_url(
            "https://example.com/api/runtime/plugins/publish?token=x",
            &package
        )
        .is_err()
    );
    assert_eq!(
        publish_url(DEFAULT_PUBLISH_URL, &package)?.as_str(),
        DEFAULT_PUBLISH_URL
    );
    assert_eq!(
        publish_url("https://example.com", &package)?.as_str(),
        "https://example.com/api/runtime/plugins/publish"
    );
    let endpoint = publish_url(
        "http://127.0.0.1:8080/api/runtime/plugins/publish",
        &package,
    )?;
    assert!(publish_job_url(endpoint.clone(), "../tokens").is_err());
    assert_eq!(
        publish_job_url(endpoint, "job-42")?.as_str(),
        "http://127.0.0.1:8080/api/runtime/publish-jobs/job-42"
    );
    Ok(())
}

#[test]
fn posts_v2_bundle_with_source_credential_and_accepts_sync_publication() -> Result<()> {
    let expected = bundle()?;
    let package = PreparedRelease::Bundle(expected.clone());
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        let (headers, body) = read_request(&mut stream)?;
        assert!(headers.starts_with("POST /api/runtime/components/publish HTTP/1.1\r\n"));
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("content-type: application/vnd.aio.component+gzip\r\n"));
        assert!(headers.contains("authorization: bearer source-bound-token\r\n"));
        assert!(!headers.contains("content-encoding:"));
        let uploaded = Bundle::decode(&body)?;
        assert_eq!(uploaded.git, expected.git);
        assert_eq!(uploaded.digest, expected.digest);
        let verified = uploaded.verify()?;
        assert_eq!(verified.component().len(), 64);
        assert_eq!(verified.frontend_files().count(), 1);
        assert_eq!(verified.migrations().count(), 1);
        respond_component(&mut stream, &expected.digest, "published")
    });
    let active = publish_to(&package, &endpoint, "source-bound-token")?;
    assert_eq!(active.revision, package.revision());
    assert!(
        active
            .detail
            .contains("d42f7f8b-8d5c-4a91-bf3f-2f29bb6fecc2")
    );
    server.join().map_err(|_| anyhow!("发布测试线程失败"))??;
    Ok(())
}

#[test]
fn v2_uses_same_host_with_component_endpoint() -> Result<()> {
    let package = PreparedRelease::Bundle(bundle()?);
    for endpoint in [
        "https://aio.addzero.site",
        DEFAULT_PUBLISH_URL,
        "https://aio.addzero.site/api/runtime/components/publish",
    ] {
        assert_eq!(
            publish_url(endpoint, &package)?.as_str(),
            "https://aio.addzero.site/api/runtime/components/publish"
        );
    }
    assert_eq!(
        publish_url("https://example.com/custom/plugins/publish", &package)?.as_str(),
        "https://example.com/custom/components/publish"
    );
    assert!(publish_url("https://example.com/other", &package).is_err());
    assert!(
        publish_url(
            "http://example.com/api/runtime/components/publish",
            &package
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn v2_rejects_wrong_revision_and_nonpublished_response() -> Result<()> {
    for (revision, state, expected_error) in [
        ("0".repeat(64), "published", "不一致的内容版本"),
        (bundle()?.digest, "failed", "组件发布状态异常"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let server = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            respond_component(&mut stream, &revision, state)
        });
        let package = PreparedRelease::Bundle(bundle()?);
        let error = publish_to(&package, &endpoint, "source-bound-token")
            .err()
            .context("无效响应必须失败")?;
        assert!(error.to_string().contains(expected_error));
        server.join().map_err(|_| anyhow!("发布测试线程失败"))??;
    }
    Ok(())
}

#[test]
fn v2_source_credential_failure_preserves_existing_bundle() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("plugin.aio-plugin");
    let bytes = bundle()?.encode()?;
    std::fs::write(&path, &bytes)?;
    let package = read_release(&path, None, None)?;
    let expected_source = package.git().to_owned();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        let (headers, body) = read_request(&mut stream)?;
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer wrong-source-token\r\n")
        );
        assert_eq!(Bundle::decode(&body)?.git, expected_source);
        respond_json(
            &mut stream,
            "403 Forbidden",
            r#"{"error":"发布凭证与来源不一致"}"#,
        )
    });
    let error = publish_to(&package, &endpoint, "wrong-source-token")
        .err()
        .context("来源凭证失败必须返回错误")?;
    assert!(error.to_string().contains("403 Forbidden"));
    assert!(error.to_string().contains("发布凭证与来源不一致"));
    assert_eq!(std::fs::read(path)?, bytes);
    server.join().map_err(|_| anyhow!("发布测试线程失败"))??;
    Ok(())
}

fn package() -> Result<PluginPackage> {
    PluginPackage::new(
        "https://example.com/plugin.git".to_owned(), "1.0.0".to_owned(), None,
        "[plugin.runtime]\nkind='page-definition'\nartifact='pages.json'\n[plugin.marketplace]\ntitle='Pages'\nsummary='Demo pages'\nlicense='MIT'\ntags=['test']\n".to_owned(),
        b"[]",
        Default::default(),
    )
}

fn bundle() -> Result<Bundle> {
    use std::fs;

    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut binary = vec![0; 64];
    binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
    binary[18] = 62;
    fs::create_dir_all(root.join("dist/frontend"))?;
    fs::create_dir_all(root.join("backend/migrations"))?;
    fs::write(
        root.join("aio-plugin.toml"),
        format!(
            "schema_version=2\n[plugin.marketplace]\ntitle='VibeCLI'\nsummary='Demo'\nlicense='MIT'\ntags=['test']\n[plugin.runtime]\nartifact='dist/server'\nhost_version='>=2026.9.21'\n[plugin.runtime.process]\nimage='sha256:{}'\n[plugin.frontend]\npath='dist/frontend'\n[plugin.database]\nmigrations='backend/migrations'\n[plugin.capabilities]\ndatabase=true\n",
            "a".repeat(64)
        ),
    )?;
    fs::write(root.join("dist/server"), binary)?;
    fs::write(
        root.join("dist/frontend/index.html"),
        "<html>VibeCLI</html>",
    )?;
    fs::write(
        root.join("backend/migrations/0001_projects.sql"),
        "CREATE TABLE projects(id INTEGER);",
    )?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        "https://example.com/vibecli.git".into(),
        "a".repeat(40),
        "1.0.0".into(),
    )
}

fn read_request(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request = Vec::new();
    let (header_end, content_length) = loop {
        let mut chunk = [0_u8; 8192];
        let read = std::io::Read::read(stream, &mut chunk)?;
        ensure!(read > 0, "发布测试请求提前结束");
        request.extend_from_slice(&chunk[..read]);
        ensure!(request.len() <= 1024 * 1024, "发布测试请求过大");
        if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let header_end = header_end + 4;
            let headers = std::str::from_utf8(&request[..header_end])?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>())
                })
                .transpose()?
                .unwrap_or(0);
            break (header_end, content_length);
        }
    };
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 8192];
        let read = std::io::Read::read(stream, &mut chunk)?;
        ensure!(read > 0, "发布测试请求正文提前结束");
        request.extend_from_slice(&chunk[..read]);
    }
    Ok((
        String::from_utf8(request[..header_end].to_vec())?,
        request[header_end..header_end + content_length].to_vec(),
    ))
}

fn respond(stream: &mut TcpStream, revision: &str, state: &str) -> Result<()> {
    let response = serde_json::json!({"data": {"job_id": "job-1", "revision": revision, "page_count": 1, "state": state, "detail": "health result"}}).to_string();
    respond_json(stream, "200 OK", &response)
}

fn respond_component(stream: &mut TcpStream, revision: &str, state: &str) -> Result<()> {
    let response = serde_json::json!({"data":{"source_id":"d42f7f8b-8d5c-4a91-bf3f-2f29bb6fecc2","revision":revision,"state":state}}).to_string();
    respond_json(stream, "200 OK", &response)
}

fn respond_json(stream: &mut TcpStream, status: &str, response: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
        response.len()
    )?;
    Ok(())
}
