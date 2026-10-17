//! 安装清单、宿主与进程 SDK 共用的匿名只读路由契约。
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_PUBLIC_BYTES: u64 = 128 * 1024 * 1024;
pub const PUBLIC_TIMEOUT_SECONDS: u64 = 120;

/// 静态段与命名参数组成的完整路径；不支持前缀通配或写入方法。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicRoute {
    pub path: String,
    pub content_types: Vec<String>,
    pub max_bytes: u64,
}

fn parameter(segment: &str) -> Option<&str> {
    segment.strip_prefix('{')?.strip_suffix('}')
}

/// URL 解码之后的资源路径；禁止双重编码、路径归一化与控制字符产生歧义。
pub fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 2048
        && path != "/"
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.~".contains(&b))
        && path[1..]
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}

impl PublicRoute {
    /// 调用前按共享契约验证完整路径，只接受 GET 和 HEAD。
    pub fn allows(&self, method: &str, path: &str) -> bool {
        matches!(method, "GET" | "HEAD")
            && valid_path(path)
            && self.path.split('/').count() == path.split('/').count()
            && self
                .path
                .split('/')
                .zip(path.split('/'))
                .all(|(pattern, actual)| parameter(pattern).is_some() || pattern == actual)
    }
}

/// 清单载入时验证数量、参数和重叠路由；SDK 也验证宿主配置快照。
pub fn validate_routes(routes: &[PublicRoute]) -> Result<()> {
    ensure!(routes.len() <= 16, "公开路由超过配额");
    for (index, route) in routes.iter().enumerate() {
        ensure!(
            route.path.starts_with('/') && route.path.len() <= 512 && route.path != "/",
            "公开路由路径无效"
        );
        let mut names = std::collections::HashSet::new();
        for segment in route.path[1..].split('/') {
            if let Some(name) = parameter(segment) {
                ensure!(
                    !name.is_empty()
                        && name.len() <= 32
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        && names.insert(name),
                    "公开路由参数无效或重复"
                );
            } else {
                ensure!(valid_path(&format!("/{segment}")), "公开路由静态路径无效");
            }
        }
        ensure!(
            (1..=MAX_PUBLIC_BYTES).contains(&route.max_bytes),
            "公开资源大小上限无效"
        );
        ensure!(
            !route.content_types.is_empty() && route.content_types.len() <= 16,
            "公开资源类型数量无效"
        );
        let mut types = std::collections::HashSet::new();
        for mime in &route.content_types {
            let segments: Vec<_> = mime.split('/').collect();
            ensure!(
                mime.len() <= 128
                    && segments.len() == 2
                    && segments.iter().all(|s| !s.is_empty()
                        && s.bytes().all(|b| b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || b".-+".contains(&b)))
                    && types.insert(mime),
                "公开资源类型无效或重复"
            );
        }
        for previous in &routes[..index] {
            let overlaps = previous.path.split('/').count() == route.path.split('/').count()
                && previous
                    .path
                    .split('/')
                    .zip(route.path.split('/'))
                    .all(|(a, b)| a == b || parameter(a).is_some() || parameter(b).is_some());
            ensure!(!overlaps, "公开路由有重叠声明");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(path: &str) -> PublicRoute {
        PublicRoute {
            path: path.into(),
            content_types: vec!["image/png".into()],
            max_bytes: MAX_PUBLIC_BYTES,
        }
    }

    #[test]
    fn routes_match_complete_paths_and_reject_ambiguous_declarations() -> Result<()> {
        let image = route("/images/{token}");
        validate_routes(std::slice::from_ref(&image))?;
        assert!(image.allows("GET", "/images/a-b_0"));
        assert!(image.allows("HEAD", "/images/a"));
        for path in [
            "/images/",
            "/images/a/extra",
            "/images/%2e%2e",
            "/images/..",
            "//images/a",
            "/images/\\a",
            "/other/a",
        ] {
            assert!(!image.allows("GET", path), "{path}");
        }
        assert!(!image.allows("POST", "/images/a"));
        for path in ["/images/{other}", "/{folder}/a", "/images/static"] {
            assert!(validate_routes(&[image.clone(), route(path)]).is_err());
        }
        for path in [
            "/",
            "/images/",
            "/{id}/{id}",
            "/{}/x",
            "/images/*",
            "/a/../b",
        ] {
            assert!(validate_routes(&[route(path)]).is_err(), "{path}");
        }
        let mut invalid = image;
        for mime in [
            "image/*",
            "image/png; charset=utf-8",
            "IMAGE/PNG",
            "text/html\r\n",
        ] {
            invalid.content_types = vec![mime.into()];
            assert!(validate_routes(&[invalid.clone()]).is_err());
        }
        invalid.content_types = vec!["image/png".into()];
        invalid.max_bytes = MAX_PUBLIC_BYTES + 1;
        assert!(validate_routes(&[invalid]).is_err());
        Ok(())
    }
}
