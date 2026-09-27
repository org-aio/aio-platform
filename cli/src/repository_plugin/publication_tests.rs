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
    let server_expected = expected.clone();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!(
        "http://{}/api/runtime/plugins/publish",
        listener.local_addr()?
    );
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
    assert_eq!(
        publish_url("https://example.com", &package)?.as_str(),
        "https://example.com/api/runtime/plugins/publish"
    );
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
fn posts_v2_component_to_component_endpoint_and_accepts_sync_response() -> Result<()> {
    let package = PreparedRelease::Bundle(bundle()?);
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let expected = package.revision().to_owned();
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        let (headers, body) = read_request(&mut stream)?;
        assert!(headers.starts_with("POST /api/runtime/components/publish HTTP/1.1\r\n"));
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: application/vnd.aio.component+gzip\r\n")
        );
        assert!(Bundle::decode(&body).is_ok());
        let response = serde_json::json!({"data":{"source_id":"d42f7f8b-8d5c-4a91-bf3f-2f29bb6fecc2","revision":expected,"state":"published"}}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
            response.len()
        )?;
        Ok(())
    });
    let active = publish_to(&package, &endpoint, "test-token")?;
    assert_eq!(active.revision, package.revision());
    assert!(active.detail.contains("d42f7f8b"));
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
    let manifest = format!(
        "schema_version=2\n[plugin.marketplace]\ntitle='Topcoat'\nsummary='Demo'\nlicense='MIT'\ntags=['test']\n[plugin.runtime]\nartifact='dist/server'\nhost_version='>=2026.9.21'\n[plugin.runtime.process]\nimage='sha256:{}'\nendpoints=[]\nhttp_endpoints=[]\nservices=[]\nworker_capabilities=[]\n[plugin.frontend]\npath='dist/frontend'\n",
        "a".repeat(64)
    );
    fs::create_dir_all(root.join("dist/frontend"))?;
    fs::write(root.join("aio-plugin.toml"), manifest)?;
    fs::write(root.join("dist/server"), binary)?;
    fs::write(
        root.join("dist/frontend/index.html"),
        "<html>Topcoat</html>",
    )?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        "https://example.com/plugin.git".into(),
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
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
        response.len()
    )?;
    Ok(())
}
