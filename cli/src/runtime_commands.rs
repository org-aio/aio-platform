use std::{
    env, fs,
    io::{self, Read, Write},
    net::IpAddr,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use reqwest::{
    Method, Url,
    blocking::{Client, Response},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct Configuration {
    schema_version: u32,
    #[serde(flatten)]
    connection: Connection,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
enum Connection {
    Direct {
        base_url: String,
    },
    Aio {
        origin: String,
        source: String,
        project: String,
    },
}

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    commands: Vec<CatalogCommand>,
}

#[derive(Deserialize)]
struct CatalogCommand {
    path: Vec<String>,
    description: String,
}

#[derive(Serialize)]
struct Invocation<'a> {
    argv: &'a [String],
}

#[derive(Deserialize)]
struct InvocationResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

pub(super) fn run(arguments: &[String]) -> Result<()> {
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["help"] | ["--help"] | ["-h"] => {
            println!("{}", usage());
            Ok(())
        }
        ["connect", base] => {
            let base = parse_base(base)?;
            connect(Connection::Direct {
                base_url: base.to_string(),
            })
        }
        ["connect", origin, "--source", source, "--project", project] => {
            let origin = hosting::parse_origin(origin)?
                .origin()
                .ascii_serialization();
            let source = uuid::Uuid::parse_str(source)
                .context("source 必须是 UUID")?
                .to_string();
            let project = uuid::Uuid::parse_str(project)
                .context("project 必须是 UUID")?
                .to_string();
            connect(Connection::Aio {
                origin,
                source,
                project,
            })
        }
        ["login", origin, "--account", account, "--password-stdin"] => {
            hosting::login(origin, account)
        }
        ["disconnect"] => {
            match fs::remove_file(configuration_path()?) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("删除 vibecli 配置失败"),
            }
            println!("已删除当前项目的 vibecli 连接配置");
            Ok(())
        }
        _ => bail!("{}", usage()),
    }
}

pub(super) fn usage() -> &'static str {
    "用法:\n  aio vibecli connect <base-url>\n  aio vibecli connect <origin> --source <UUID> --project <UUID>\n  aio vibecli login <origin> --account <account> --password-stdin\n  aio vibecli disconnect"
}

pub(super) fn print_help() -> Result<()> {
    let Some(connection) = configured_connection()? else {
        return Ok(());
    };
    let catalog = catalog(&connection)?;
    if !catalog.commands.is_empty() {
        println!("\n远端命令：");
        for command in catalog.commands {
            println!("  aio {}  {}", command.path.join(" "), command.description);
        }
    }
    Ok(())
}

// 内置命令已在主入口处理；剩余 argv 完整交给服务端解析。
pub(super) fn invoke(arguments: &[String]) -> Result<Option<i32>> {
    let Some(connection) = configured_connection()? else {
        return Ok(None);
    };
    let request = Invocation { argv: arguments };
    let result: InvocationResult =
        request_json(&connection, "invoke", Method::POST, Some(&request))?;
    ensure!(
        (0..=255).contains(&result.exit_code),
        "vibecli exit_code 必须在 0..255 范围内"
    );
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(result.stdout.as_bytes())
        .context("写入远端命令标准输出失败")?;
    stdout.flush().context("刷新远端命令标准输出失败")?;
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(result.stderr.as_bytes())
        .context("写入远端命令错误输出失败")?;
    stderr.flush().context("刷新远端命令错误输出失败")?;
    Ok(Some(result.exit_code))
}

fn configuration_path() -> Result<PathBuf> {
    let directory = env::current_dir().context("无法定位当前项目目录")?;
    Ok(directory.join(".aio/vibecli.json"))
}

fn connect(connection: Connection) -> Result<()> {
    let catalog = catalog(&connection)?;
    let configuration = Configuration {
        schema_version: 1,
        connection,
    };
    let bytes = serde_json::to_vec_pretty(&configuration)?;
    let path = configuration_path()?;
    let directory = path.parent().context("vibecli 配置目录不存在")?;
    fs::create_dir_all(directory).context("创建 vibecli 配置目录失败")?;
    let temporary = directory.join(format!(".vibecli-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, bytes).context("写入 vibecli 配置失败")?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("保存 vibecli 配置失败");
    }
    println!("已连接 vibecli，发现 {} 个命令", catalog.commands.len());
    Ok(())
}

fn configured_connection() -> Result<Option<Connection>> {
    if let Some(value) = env::var_os("AIO_VIBECLI_URL") {
        let value = value.to_str().context("AIO_VIBECLI_URL 必须是 UTF-8")?;
        let base = parse_base(value)?;
        return Ok(Some(Connection::Direct {
            base_url: base.to_string(),
        }));
    }
    let bytes = match fs::read(configuration_path()?) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("读取 vibecli 配置失败"),
    };
    let configuration: Configuration =
        serde_json::from_slice(&bytes).context("vibecli 配置格式无效")?;
    ensure!(
        configuration.schema_version == 1,
        "不支持的 vibecli 配置 schema_version"
    );
    Ok(Some(configuration.connection))
}

fn parse_base(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow::anyhow!("vibecli 服务地址无效"))?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "vibecli 服务地址不能包含凭据、query 或 fragment"
    );
    let host = url.host_str().context("vibecli 服务地址缺少主机")?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "vibecli 仅允许 HTTPS 或回环 HTTP 地址"
    );
    Ok(url)
}

fn catalog(connection: &Connection) -> Result<Catalog> {
    let mut catalog: Catalog =
        request_json::<_, Invocation<'_>>(connection, "catalog", Method::GET, None)?;
    ensure!(
        catalog.schema_version == 1,
        "不支持的 vibecli catalog schema_version"
    );
    for command in &mut catalog.commands {
        ensure!(
            !command.path.is_empty()
                && command.path.iter().all(|segment| !segment.is_empty()
                    && !segment
                        .chars()
                        .any(|character| character.is_whitespace() || character.is_control())),
            "vibecli catalog 命令路径或描述无效"
        );
        command.description = command
            .description
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
    }
    Ok(catalog)
}

fn request_json<T: DeserializeOwned, P: Serialize>(
    connection: &Connection,
    action: &str,
    method: Method,
    payload: Option<&P>,
) -> Result<T> {
    let base = match connection {
        Connection::Direct { base_url } => parse_base(base_url)?,
        Connection::Aio {
            origin,
            source,
            project,
        } => {
            return hosting::request(origin, source, project, action, method, payload);
        }
    };
    let mut url = base;
    // 使用路径段追加，避免代理前缀被 Url::join 的绝对路径覆盖。
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("vibecli 地址不支持路径"))?
        .pop_if_empty()
        .push(action);
    let client = http_client()?;
    let mut request = client.request(method, url);
    let token = match env::var("AIO_VIBECLI_TOKEN") {
        Ok(token) => Some(token),
        Err(env::VarError::NotPresent) => None,
        Err(_) => bail!("AIO_VIBECLI_TOKEN 必须是 UTF-8"),
    };
    if let Some(token) = &token {
        request = request.bearer_auth(token);
    }
    if let Some(payload) = payload {
        request = request.json(payload);
    }
    let response = request.send().context("请求 vibecli 服务失败")?;
    let secrets = token.as_deref().into_iter().collect::<Vec<_>>();
    let bytes = response_bytes(response, &secrets)?;
    decode_json(&bytes)
}

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("创建 vibecli HTTP 客户端失败")
}

fn response_bytes(response: Response, secrets: &[&str]) -> Result<Vec<u8>> {
    let status = response.status();
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_RESPONSE_BYTES),
        "vibecli 响应超过 1 MiB 限制（HTTP {status}）"
    );
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("读取 vibecli 响应失败"))?;
    ensure!(
        bytes.len() as u64 <= MAX_RESPONSE_BYTES,
        "vibecli 响应超过 1 MiB 限制（HTTP {status}）"
    );
    if !status.is_success() {
        remote_error(status.as_u16(), &bytes, secrets)?;
    }
    Ok(bytes)
}

fn remote_error(status: u16, bytes: &[u8], secrets: &[&str]) -> Result<()> {
    let mut message = String::from_utf8_lossy(bytes).into_owned();
    for secret in secrets.iter().filter(|value| !value.is_empty()) {
        message = message.replace(secret, "[redacted]");
    }
    let message = message
        .chars()
        .filter(|character| !character.is_control())
        .take(300)
        .collect::<String>();
    bail!("vibecli 服务返回 HTTP {status}：{message}")
}

fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("vibecli 响应 JSON 格式无效"))
}
mod hosting;
