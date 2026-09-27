use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use tempfile::tempdir;

type MockResponse = (u16, String, Vec<(String, String)>);

struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    cookie: Option<String>,
    body: Value,
}

struct Server {
    base: String,
    requests: mpsc::Receiver<Request>,
    handle: thread::JoinHandle<Result<()>>,
}

impl Server {
    fn start(prefix: &str, responses: Vec<(u16, String)>) -> Result<Self> {
        let responses = responses
            .into_iter()
            .map(|(status, body)| (status, body, Vec::new()))
            .collect();
        Self::start_with_headers(prefix, responses)
    }

    fn start_with_headers(prefix: &str, responses: Vec<MockResponse>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}{prefix}", listener.local_addr()?);
        let (sender, requests) = mpsc::channel();
        let handle = thread::spawn(move || {
            for (status, body, headers) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            ensure!(std::time::Instant::now() < deadline, "模拟服务等待请求超时");
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                sender
                    .send(read_request(&mut stream)?)
                    .context("记录模拟请求失败")?;
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                )?;
                for (name, value) in headers {
                    write!(stream, "{name}: {value}\r\n")?;
                }
                if let Err(error) = write!(stream, "\r\n{body}") {
                    // CLI 根据 Content-Length 提前拒绝超限响应，服务端写入可能被对端中止。
                    ensure!(
                        body.len() > 1024 * 1024
                            && matches!(
                                error.kind(),
                                std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::ConnectionReset
                            ),
                        "写入模拟响应失败: {error}"
                    );
                }
            }
            Ok(())
        });
        Ok(Self {
            base,
            requests,
            handle,
        })
    }

    fn request(&self) -> Result<Request> {
        self.requests
            .recv_timeout(Duration::from_secs(15))
            .context("没有收到模拟请求")
    }

    fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| anyhow::anyhow!("模拟服务线程异常"))?
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        ensure!(bytes.len() < 64 * 1024, "请求头过大");
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])?;
    let first = headers.lines().next().context("缺少请求行")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().context("缺少请求方法")?.to_owned();
    let path = parts.next().context("缺少请求路径")?.to_owned();
    let mut length = 0;
    let mut authorization = None;
    let mut cookie = None;
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse()?;
        }
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_owned());
        }
        if name.eq_ignore_ascii_case("cookie") {
            cookie = Some(value.trim().to_owned());
        }
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)?
    };
    Ok(Request {
        method,
        path,
        authorization,
        cookie,
        body,
    })
}

fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aio"));
    command
        .current_dir(root)
        .args(args)
        .env_remove("AIO_VIBECLI_URL")
        .env_remove("AIO_VIBECLI_TOKEN")
        .env("AIO_VIBECLI_SESSION_FILE", root.join("session.json"));
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(key);
    }
    command
}

fn successful(output: Output) -> Result<String> {
    ensure!(
        output.status.success(),
        "CLI 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn catalog() -> String {
    json!({"schema_version":1,"revision":"r1","commands":[{"path":["greet","person"],"description":"问候"}]}).to_string()
}

#[test]
fn multiline_descriptions_are_normalized_for_terminal_help() -> Result<()> {
    let root = tempdir()?;
    let description = json!({"schema_version":1,"commands":[{"path":["greet"],"description":"first\nsecond\tline\u{1b}"}]}).to_string();
    let server = Server::start("", vec![(200, description.clone()), (200, description)])?;
    successful(command(root.path(), &["vibecli", "connect", &server.base]).output()?)?;
    server.request()?;
    let help = successful(command(root.path(), &["--help"]).output()?)?;
    assert!(help.contains("aio greet  first second line"));
    assert!(!help.contains('\u{1b}'));
    server.request()?;
    server.finish()
}

#[test]
fn connects_forwards_arguments_help_and_exit_code_with_proxy_prefix() -> Result<()> {
    let root = tempdir()?;
    let token = "private-test-token";
    let server = Server::start(
        "/api/runtime/vibecli/",
        vec![
            (200, catalog()),
            (200, json!({"schema_version":1,"revision":"r2","commands":[{"path":["new-command"],"description":"刚发布的命令"},{"path":["greet","person"],"description":"问候"}]}).to_string()),
            (
                200,
                json!({"stdout":"result\n","stderr":"warning\n","exit_code":7,"revision":"r2"})
                    .to_string(),
            ),
            (
                200,
                json!({"stdout":"remote help\n","stderr":"","exit_code":0}).to_string(),
            ),
        ],
    )?;
    let connect = command(root.path(), &["vibecli", "connect", &server.base])
        .env("AIO_VIBECLI_TOKEN", token)
        .output()?;
    successful(connect)?;
    let request = server.request()?;
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/runtime/vibecli/catalog");
    assert_eq!(
        request.authorization.as_deref(),
        Some("Bearer private-test-token")
    );
    let bytes = fs::read(root.path().join(".aio/vibecli.json"))?;
    let configuration: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        configuration,
        json!({"schema_version":1,"transport":"direct","base_url":server.base})
    );
    assert!(!String::from_utf8(bytes)?.contains(token));

    let help = successful(command(root.path(), &["--help"]).output()?)?;
    assert!(help.contains("aio greet person  问候"));
    assert!(help.contains("aio new-command"));
    assert_eq!(server.request()?.path, "/api/runtime/vibecli/catalog");
    let args = [
        "greet",
        "person",
        "--name",
        "张 三",
        "--number=-3",
        "--",
        "literal",
    ];
    let output = command(root.path(), &args)
        .env("AIO_VIBECLI_TOKEN", token)
        .output()?;
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"result\n");
    assert_eq!(output.stderr, b"warning\n");
    let request = server.request()?;
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/runtime/vibecli/invoke");
    assert_eq!(request.body, json!({"argv":args}));
    assert_eq!(
        request.authorization.as_deref(),
        Some("Bearer private-test-token")
    );

    assert_eq!(
        successful(command(root.path(), &["greet", "--help"]).output()?)?,
        "remote help\n"
    );
    assert_eq!(server.request()?.body, json!({"argv":["greet","--help"]}));
    server.finish()?;
    successful(command(root.path(), &["vibecli", "disconnect"]).output()?)?;
    assert!(!root.path().join(".aio/vibecli.json").exists());
    Ok(())
}

#[test]
fn environment_endpoint_takes_precedence_and_builtins_stay_local() -> Result<()> {
    let root = tempdir()?;
    fs::create_dir(root.path().join(".aio"))?;
    fs::write(
        root.path().join(".aio/vibecli.json"),
        "invalid project configuration",
    )?;
    let server = Server::start(
        "/remote",
        vec![(
            200,
            json!({"stdout":"environment\n","stderr":"","exit_code":0}).to_string(),
        )],
    )?;
    let output = command(root.path(), &["custom", "--flag"])
        .env("AIO_VIBECLI_URL", &server.base)
        .output()?;
    assert_eq!(successful(output)?, "environment\n");
    assert_eq!(server.request()?.body, json!({"argv":["custom","--flag"]}));
    server.finish()?;
    let version = successful(
        command(root.path(), &["--version"])
            .env("AIO_VIBECLI_URL", "invalid")
            .output()?,
    )?;
    assert!(version.starts_with("aio "));
    let tool = successful(
        command(root.path(), &["tool", "capabilities"])
            .env("AIO_VIBECLI_URL", "invalid")
            .output()?,
    )?;
    assert!(tool.contains("\"install\":true"));
    Ok(())
}

#[test]
fn no_configuration_keeps_local_help_and_unknown_command_errors() -> Result<()> {
    let root = tempdir()?;
    for args in [&["help"][..], &["--help"][..], &["-h"][..], &[][..]] {
        let help = successful(command(root.path(), args).output()?)?;
        assert!(help.contains("aio plugin init"));
        assert!(!help.contains("远端命令"));
    }
    let output = command(root.path(), &["unregistered"]).output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("未知命令: unregistered"));
    Ok(())
}

#[test]
fn rejects_unsafe_urls_and_invalid_catalog_without_persisting() -> Result<()> {
    let root = tempdir()?;
    for base in [
        "http://example.com",
        "file:///tmp/service",
        "https://user:password@example.com",
        "https://example.com?token=secret",
        "https://example.com#fragment",
    ] {
        let output = command(root.path(), &["vibecli", "connect", base]).output()?;
        assert!(!output.status.success());
        assert!(!root.path().join(".aio/vibecli.json").exists());
        assert!(!String::from_utf8(output.stderr)?.contains("password"));
    }
    for body in [
        json!({"schema_version":2,"commands":[]}),
        json!({"schema_version":1,"commands":[{"path":[],"description":"bad"}]}),
    ] {
        let server = Server::start("", vec![(200, body.to_string())])?;
        let output = command(root.path(), &["vibecli", "connect", &server.base]).output()?;
        assert!(!output.status.success());
        assert!(!root.path().join(".aio/vibecli.json").exists());
        server.request()?;
        server.finish()?;
    }
    Ok(())
}

#[test]
fn limits_responses_and_redacts_token_from_remote_errors() -> Result<()> {
    let root = tempdir()?;
    let token = "do-not-disclose";
    let server = Server::start("", vec![(403, format!("denied {token}"))])?;
    let output = command(root.path(), &["private"])
        .env("AIO_VIBECLI_URL", &server.base)
        .env("AIO_VIBECLI_TOKEN", token)
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("403"));
    assert!(error.contains("[redacted]"));
    assert!(!error.contains(token));
    assert!(!String::from_utf8(output.stdout)?.contains(token));
    server.request()?;
    server.finish()?;

    let server = Server::start("", vec![(200, "x".repeat(1024 * 1024 + 1))])?;
    let output = command(root.path(), &["private"])
        .env("AIO_VIBECLI_URL", &server.base)
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("1 MiB"), "{error}");
    server.request()?;
    server.finish()?;
    Ok(())
}

fn component(body: Value) -> String {
    let body = body.to_string().into_bytes();
    json!({"data":{"status":200,"headers":[],"body":body}}).to_string()
}

#[test]
fn logs_in_and_invokes_through_aio_mount_with_private_session() -> Result<()> {
    let root = tempdir()?;
    let session_secret = "private-session-token";
    let password = "private-test-password";
    let source = "37e77f55-4210-4b73-a974-9d19d8c99f21";
    let project = "47937056-ad6e-4d0b-b09b-91c2cd2f53a4";
    let mount = json!({"data":{"abi":2,"token":"grant-token"}}).to_string();
    let server = Server::start_with_headers(
        "",
        vec![
            (
                200,
                json!({"data":{"user_id":"user","tenant_id":"tenant"}}).to_string(),
                vec![(
                    "Set-Cookie".into(),
                    format!("aio_session={session_secret}; HttpOnly; SameSite=Lax"),
                )],
            ),
            (200, mount.clone(), Vec::new()),
            (
                200,
                component(serde_json::from_str(&catalog())?),
                Vec::new(),
            ),
            (200, mount.clone(), Vec::new()),
            (
                200,
                component(serde_json::from_str(&catalog())?),
                Vec::new(),
            ),
            (200, mount, Vec::new()),
            (
                200,
                component(json!({"stdout":"aio result\n","stderr":"aio warning\n","exit_code":9})),
                Vec::new(),
            ),
        ],
    )?;
    let mut child = command(
        root.path(),
        &[
            "vibecli",
            "login",
            &server.base,
            "--account",
            "example",
            "--password-stdin",
        ],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
    let mut stdin = child.stdin.take().context("缺少登录密码输入管道")?;
    writeln!(stdin, "{password}")?;
    drop(stdin);
    let login_output = successful(child.wait_with_output()?)?;
    assert!(!login_output.contains(password));
    assert!(!login_output.contains(session_secret));
    let login = server.request()?;
    assert_eq!(login.path, "/api/auth/login");
    assert_eq!(login.body, json!({"account":"example","password":password}));
    let session_path = root.path().join("session.json");
    let session = fs::read_to_string(&session_path)?;
    assert!(!session.contains(password));
    assert!(session.contains("aio_session=private-session-token"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&session_path)?.permissions().mode() & 0o777,
            0o600
        );
    }

    successful(
        command(
            root.path(),
            &[
                "vibecli",
                "connect",
                &server.base,
                "--source",
                source,
                "--project",
                project,
            ],
        )
        .output()?,
    )?;
    let config = fs::read_to_string(root.path().join(".aio/vibecli.json"))?;
    assert_eq!(
        serde_json::from_str::<Value>(&config)?,
        json!({"schema_version":1,"transport":"aio","origin":server.base,"source":source,"project":project})
    );
    assert!(!config.contains(session_secret));
    assert!(!config.contains(password));
    let help = successful(command(root.path(), &["help"]).output()?)?;
    assert!(help.contains("aio greet person"));
    let args = ["greet", "person", "--name", "example"];
    let result = command(root.path(), &args).output()?;
    assert_eq!(result.status.code(), Some(9));
    assert_eq!(result.stdout, b"aio result\n");
    assert_eq!(result.stderr, b"aio warning\n");
    for action in ["catalog", "catalog", "invoke"] {
        let mount = server.request()?;
        assert_eq!(mount.method, "POST");
        assert_eq!(mount.path, "/api/runtime/frontend/mount");
        assert_eq!(
            mount.body,
            json!({"page_id":format!("component:{source}:vibecli")})
        );
        assert_eq!(
            mount.cookie.as_deref(),
            Some("aio_session=private-session-token")
        );
        assert!(mount.authorization.is_none());
        let request = server.request()?;
        assert_eq!(request.path, "/api/runtime/components/grant-token/request");
        assert_eq!(
            request.cookie.as_deref(),
            Some("aio_session=private-session-token")
        );
        let body = if action == "invoke" {
            serde_json::to_vec(&json!({"argv":args}))?
        } else {
            Vec::new()
        };
        assert_eq!(
            request.body,
            json!({"method":if action=="catalog" {"GET"} else {"POST"},"path":format!("/api/cli/{project}/{action}"),"query":null,"headers":[{"name":"content-type","value":"application/json"}],"body":body})
        );
    }
    server.finish()?;
    Ok(())
}

#[test]
fn keeps_static_help_when_remote_fails_and_rejects_invalid_exit_code() -> Result<()> {
    let root = tempdir()?;
    let server = Server::start(
        "",
        vec![
            (503, "temporarily unavailable".into()),
            (
                200,
                json!({"stdout":"invalid","stderr":"","exit_code":256}).to_string(),
            ),
        ],
    )?;
    let output = command(root.path(), &["-h"])
        .env("AIO_VIBECLI_URL", &server.base)
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("aio plugin init"));
    assert!(String::from_utf8(output.stderr)?.contains("503"));
    server.request()?;
    let output = command(root.path(), &["custom"])
        .env("AIO_VIBECLI_URL", &server.base)
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("0..255"));
    server.request()?;
    server.finish()?;
    Ok(())
}

#[test]
fn rejects_invalid_aio_configuration_before_network_or_session_access() -> Result<()> {
    let root = tempdir()?;
    fs::create_dir(root.path().join(".aio"))?;
    for (source, project, expected) in [
        (
            "bad/source",
            "47937056-ad6e-4d0b-b09b-91c2cd2f53a4",
            "source 必须是 UUID",
        ),
        (
            "37e77f55-4210-4b73-a974-9d19d8c99f21",
            "bad/project",
            "project 必须是 UUID",
        ),
    ] {
        let config = json!({"schema_version":1,"transport":"aio","origin":"http://127.0.0.1:1","source":source,"project":project});
        fs::write(root.path().join(".aio/vibecli.json"), config.to_string())?;
        let output = command(root.path(), &["custom"]).output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)?.contains(expected));
    }
    let output = command(
        root.path(),
        &[
            "vibecli",
            "login",
            "http://127.0.0.1:1/prefix",
            "--account",
            "example",
            "--password-stdin",
        ],
    )
    .stdin(Stdio::null())
    .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("纯 origin"));
    assert!(!root.path().join("session.json").exists());
    Ok(())
}

#[test]
fn aio_errors_redact_session_and_grant_secrets() -> Result<()> {
    let root = tempdir()?;
    let session_secret = "secret-session-value";
    let grant_secret = "secret-grant-value";
    let error = format!("session {session_secret}, grant {grant_secret}").into_bytes();
    let server = Server::start(
        "",
        vec![
            (
                200,
                json!({"data":{"abi":2,"token":grant_secret}}).to_string(),
            ),
            (
                200,
                json!({"data":{"status":403,"headers":[],"body":error}}).to_string(),
            ),
        ],
    )?;
    let session = json!({"schema_version":1,"origin":server.base,"cookie":format!("aio_session={session_secret}")});
    let session_path = root.path().join("session.json");
    fs::write(&session_path, session.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&session_path, fs::Permissions::from_mode(0o600))?;
    }
    fs::create_dir(root.path().join(".aio"))?;
    let config = json!({"schema_version":1,"transport":"aio","origin":server.base,"source":"37e77f55-4210-4b73-a974-9d19d8c99f21","project":"47937056-ad6e-4d0b-b09b-91c2cd2f53a4"});
    fs::write(root.path().join(".aio/vibecli.json"), config.to_string())?;
    let output = command(root.path(), &["custom"]).output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("403"));
    assert!(error.contains("[redacted]"));
    assert!(!error.contains(session_secret));
    assert!(!error.contains(grant_secret));
    server.request()?;
    server.request()?;
    server.finish()?;
    Ok(())
}
