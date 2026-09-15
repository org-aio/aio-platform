use super::network;
use anyhow::{Context as _, Result, ensure};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Identity {
    pub repository: String,
    pub repository_owner: String,
    pub sha: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub event_name: String,
    pub workflow_ref: String,
}

impl Identity {
    pub(super) fn validate(&self, owner: &str) -> Result<()> {
        ensure!(
            !owner.is_empty() && self.repository_owner == owner,
            "GitHub 发布者不属于平台允许的 owner"
        );
        let (actual, repo) = self.repository.split_once('/').context("仓库身份无效")?;
        ensure!(
            actual == owner
                && !repo.is_empty()
                && repo
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "仓库身份无效"
        );
        ensure!(
            self.sha.len() == 40 && self.sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "源码 SHA 无效"
        );
        ensure!(
            ["push", "workflow_dispatch"].contains(&self.event_name.as_str()),
            "仅接受推送或手动发布工作流"
        );
        ensure!(
            self.workflow_ref
                == format!(
                    "{}/.github/workflows/aio-cli.yml@{}",
                    self.repository, self.reference
                ),
            "CLI 发布必须来自 aio-cli.yml"
        );
        Ok(())
    }
}

pub(super) async fn verify(token: &str, owner: &str) -> Result<Identity> {
    ensure!(token.len() < 20000, "发布身份过大");
    let header = decode_header(token).context("发布身份格式无效")?;
    ensure!(header.alg == Algorithm::RS256, "发布身份签名算法无效");
    let kid = header.kid.context("发布身份缺少签名标识")?;
    let keys: JwkSet = serde_json::from_slice(
        &network::bytes(
            "https://token.actions.githubusercontent.com/.well-known/jwks",
            false,
            256 * 1024,
        )
        .await?,
    )?;
    let key = keys.find(&kid).context("GitHub 签名密钥不存在")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://token.actions.githubusercontent.com"]);
    validation.set_audience(&[az_tool::OFFICIAL_ORIGIN]);
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud"]);
    let identity = decode::<Identity>(token, &DecodingKey::from_jwk(key)?, &validation)
        .context("GitHub 发布身份校验失败")?
        .claims;
    identity.validate(owner)?;
    Ok(identity)
}
