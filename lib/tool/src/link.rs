use anyhow::{Result, ensure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallLink {
    pub id: String,
    pub version: String,
}

impl InstallLink {
    pub fn parse(input: &str) -> Result<Self> {
        ensure!(
            input.len() <= 512 && !input.chars().any(char::is_control),
            "安装链接无效"
        );
        let url = url::Url::parse(input)?;
        ensure!(
            url.scheme() == "aio"
                && url.host_str() == Some("install")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "只接受 aio://install/<id>?version=<版本>"
        );
        let id = url.path().strip_prefix('/').unwrap_or_default();
        let query = url.query_pairs().collect::<Vec<_>>();
        ensure!(
            crate::validation::identifier(id) && query.len() == 1 && query[0].0 == "version",
            "安装链接必须包含工具 ID 和唯一版本参数"
        );
        let version = semver::Version::parse(&query[0].1)?;
        let result = Self {
            id: id.into(),
            version: version.to_string(),
        };
        // 只接受生成器的规范形式，拒绝转义路径和 URL 解析器归一化后的额外输入。
        ensure!(result.to_string() == input, "安装链接必须使用规范格式");
        Ok(result)
    }
}

impl std::fmt::Display for InstallLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("version", &self.version)
            .finish();
        write!(f, "aio://install/{}?{query}", self.id)
    }
}
