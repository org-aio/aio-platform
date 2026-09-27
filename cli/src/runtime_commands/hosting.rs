use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
};

use anyhow::{Context as _, Result, ensure};
use reqwest::{
    Method, Url,
    header::{COOKIE, HeaderValue, SET_COOKIE},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

use super::{
    MAX_RESPONSE_BYTES, decode_json, http_client, parse_base, remote_error, response_bytes,
};

#[derive(Deserialize, Serialize)]
struct Session {
    schema_version: u32,
    origin: String,
    cookie: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct LoginSession {
    user_id: String,
    tenant_id: String,
}

#[derive(Deserialize)]
struct Grant {
    abi: u32,
    token: String,
}

#[derive(Deserialize)]
struct ComponentResponse {
    status: u16,
    body: Vec<u8>,
}

pub(super) fn parse_origin(value: &str) -> Result<Url> {
    let origin = parse_base(value)?;
    ensure!(
        origin.path() == "/",
        "AIO 宿主地址必须是纯 origin，不能含路径前缀"
    );
    Ok(origin)
}

pub(super) fn login(origin: &str, account: &str) -> Result<()> {
    let origin = parse_origin(origin)?;
    ensure!(!account.trim().is_empty(), "登录账号不能为空");
    let mut password = String::new();
    io::stdin()
        .take(64 * 1024 + 1)
        .read_to_string(&mut password)
        .context("读取登录密码失败")?;
    ensure!(password.len() <= 64 * 1024, "登录密码超过长度限制");
    let password = password.trim_end_matches(['\r', '\n']);
    ensure!(!password.is_empty(), "登录密码不能为空");
    let payload = json!({"account":account,"password":password});
    let response = http_client()?
        .post(origin.join("/api/auth/login")?)
        .json(&payload)
        .send()
        .context("请求 AIO 登录失败")?;
    let cookie = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .find(|value| value.starts_with("aio_session="))
        .map(str::to_owned);
    let secrets = [password, cookie.as_deref().unwrap_or_default()];
    let bytes = response_bytes(response, &secrets)?;
    let cookie = cookie.context("AIO 登录响应缺少 aio_session Cookie")?;
    ensure!(cookie.len() > "aio_session=".len(), "AIO 登录响应会话为空");
    let result: Envelope<LoginSession> = decode_json(&bytes)?;
    ensure!(
        !result.data.user_id.is_empty() && !result.data.tenant_id.is_empty(),
        "AIO 登录响应缺少用户或工作区"
    );
    cookie_header(&cookie)?;
    let session = Session {
        schema_version: 1,
        origin: origin.origin().ascii_serialization(),
        cookie,
    };
    save_session(&session)?;
    println!("已保存 vibecli 的 AIO 登录会话");
    Ok(())
}

pub(super) fn request<T: DeserializeOwned, P: Serialize>(
    origin: &str,
    source: &str,
    project: &str,
    action: &str,
    method: Method,
    payload: Option<&P>,
) -> Result<T> {
    let origin = parse_origin(origin)?;
    let source = uuid::Uuid::parse_str(source).context("source 必须是 UUID")?;
    let project = uuid::Uuid::parse_str(project).context("project 必须是 UUID")?;
    let session = load_session()?;
    ensure!(
        session.origin == origin.origin().ascii_serialization(),
        "AIO 登录会话与当前宿主不符，请重新登录"
    );
    let client = http_client()?;
    let cookie = cookie_header(&session.cookie)?;
    let mount = json!({"page_id":format!("component:{source}:vibecli")});
    let response = client
        .post(origin.join("/api/runtime/frontend/mount")?)
        .header(COOKIE, cookie.clone())
        .json(&mount)
        .send()
        .context("挂载 AIO vibecli 组件失败")?;
    let session_secret = session
        .cookie
        .strip_prefix("aio_session=")
        .context("AIO 会话格式无效")?;
    let bytes = response_bytes(response, &[&session.cookie, session_secret])?;
    let grant: Envelope<Grant> = decode_json(&bytes)?;
    ensure!(
        grant.data.abi == 2
            && !grant.data.token.is_empty()
            && grant
                .data
                .token
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || b"_-".contains(&value)),
        "AIO 宿主未返回有效 v2 授权"
    );
    let body = payload
        .map(serde_json::to_vec)
        .transpose()?
        .unwrap_or_default();
    let envelope = json!({
        "method":method.as_str(),
        "path":format!("/api/cli/{project}/{action}"),
        "query":null,
        "headers":[{"name":"content-type","value":"application/json"}],
        "body":body,
    });
    let route = format!("/api/runtime/components/{}/request", grant.data.token);
    let response = client
        .post(origin.join(&route)?)
        .header(COOKIE, cookie)
        .json(&envelope)
        .send()
        .map_err(|error| anyhow::anyhow!("请求 AIO vibecli 组件失败：{}", error.without_url()))?;
    let secrets = [&session.cookie, session_secret, grant.data.token.as_str()];
    let bytes = response_bytes(response, &secrets)?;
    let response: Envelope<ComponentResponse> = decode_json(&bytes)?;
    ensure!(
        response.data.body.len() as u64 <= MAX_RESPONSE_BYTES,
        "vibecli 组件响应超过 1 MiB 限制"
    );
    if !(200..300).contains(&response.data.status) {
        remote_error(response.data.status, &response.data.body, &secrets)?;
    }
    decode_json(&response.data.body)
}

fn cookie_header(cookie: &str) -> Result<HeaderValue> {
    ensure!(
        cookie.starts_with("aio_session=")
            && cookie.len() > "aio_session=".len()
            && !cookie.contains(';'),
        "AIO 会话格式无效"
    );
    let mut value = HeaderValue::from_str(cookie).context("AIO 会话 Cookie 格式无效")?;
    value.set_sensitive(true);
    Ok(value)
}

fn session_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("AIO_VIBECLI_SESSION_FILE") {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .context("无法定位 vibecli 用户会话目录")?;
    Ok(PathBuf::from(home).join(".config/aio/vibecli-session.json"))
}

fn save_session(session: &Session) -> Result<()> {
    let path = session_path()?;
    let directory = path.parent().context("vibecli 会话文件缺少目录")?;
    fs::create_dir_all(directory).context("创建 vibecli 会话目录失败")?;
    let temporary = directory.join(format!(".vibecli-session-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .context("创建私有 vibecli 会话文件失败")?;
    let bytes = serde_json::to_vec(&session)?;
    file.write_all(&bytes).context("保存 vibecli 会话失败")?;
    file.sync_all().context("同步 vibecli 会话失败")?;
    drop(file);
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("保存 vibecli 会话失败");
    }
    Ok(())
}

fn load_session() -> Result<Session> {
    let path = session_path()?;
    let metadata =
        fs::symlink_metadata(&path).context("没有可用的 AIO 会话，请先运行 aio vibecli login")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "vibecli 会话必须是普通文件"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "vibecli 会话权限必须为 0600"
        );
    }
    ensure!(metadata.len() <= 64 * 1024, "vibecli 会话文件超过长度限制");
    let bytes = fs::read(path).context("读取 vibecli 会话失败")?;
    let session: Session = decode_json(&bytes)?;
    ensure!(
        session.schema_version == 1,
        "不支持的 vibecli 会话 schema_version"
    );
    parse_origin(&session.origin)?;
    cookie_header(&session.cookie)?;
    Ok(session)
}
