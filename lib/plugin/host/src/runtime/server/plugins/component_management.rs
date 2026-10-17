//! 管理权限由部署方按整包版本授予；租户管理员不能通过上传清单自授宿主能力。

use std::{any::TypeId, collections::HashSet, sync::OnceLock};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_core::{Plugin, PluginFuture};

use super::{ManagementPlugin, RuntimeAssembly};
use crate::runtime::server::{RuntimeState, weak_state::WeakRuntimeState};

/// 部署方认可的管理实现；摘要覆盖整包，后续版本须单独加入信任声明。
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustedManagementComponent {
    pub git: String,
    pub digest: String,
}

/// 选择可调用宿主管理接口的整包；移除此插件或撤去摘要会暂停相关安装。
pub struct ComponentManagementPlugin(pub Vec<TrustedManagementComponent>);

pub(in crate::runtime::server) struct ManagementAccess {
    trusted: HashSet<(String, String)>,
    state: OnceLock<WeakRuntimeState>,
}

impl ManagementAccess {
    fn new(declarations: &[TrustedManagementComponent]) -> Result<Self> {
        ensure!(declarations.len() <= 128, "管理整包信任声明超过限制");
        let mut trusted = HashSet::new();
        for declaration in declarations {
            let git = reqwest::Url::parse(&declaration.git).context("管理整包来源无效")?;
            ensure!(
                git.scheme() == "https"
                    && git.host_str().is_some()
                    && git.username().is_empty()
                    && git.password().is_none()
                    && git.query().is_none()
                    && git.fragment().is_none()
                    && declaration.git.len() <= 2048
                    && declaration.git.trim() == declaration.git
                    && !declaration.git.chars().any(char::is_control)
                    && declaration.digest.len() == 64
                    && declaration
                        .digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "管理整包必须声明来源和规范 SHA256 摘要"
            );
            ensure!(
                trusted.insert((declaration.git.clone(), declaration.digest.clone())),
                "管理整包信任声明重复"
            );
        }
        Ok(Self {
            trusted,
            state: OnceLock::new(),
        })
    }

    pub(in crate::runtime::server) fn trusts(&self, bundle: &VerifiedBundle) -> bool {
        bundle.git().is_some_and(|git| {
            self.trusted
                .contains(&(git.to_owned(), bundle.digest().to_owned()))
        })
    }

    pub(in crate::runtime::server) fn runtime(&self) -> Result<RuntimeState> {
        self.state.get().context("管理能力尚未初始化")?.upgrade()
    }
}

impl Plugin<RuntimeAssembly> for ComponentManagementPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<ManagementPlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.observe_catalog();
            assembly.services.add_value(ManagementAccess::new(&self.0)?);
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    ensure!(
                        state
                            .service::<ManagementAccess>()?
                            .state
                            .set(state.downgrade())
                            .is_ok(),
                        "管理能力不能重复初始化"
                    );
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_trust_rejects_ambiguous_or_unbounded_declarations() {
        let valid = TrustedManagementComponent {
            git: "https://github.com/example/manager.git".into(),
            digest: "a".repeat(64),
        };
        assert!(ManagementAccess::new(std::slice::from_ref(&valid)).is_ok());
        assert!(ManagementAccess::new(&[valid.clone(), valid.clone()]).is_err());
        assert!(ManagementAccess::new(&vec![valid.clone(); 129]).is_err());
        for git in [
            "http://github.com/example/manager.git",
            "https://user:password@github.com/example/manager.git",
            "https://github.com/example/manager.git?revision=latest",
            "https://github.com/example/manager.git#main",
            " https://github.com/example/manager.git",
        ] {
            let candidate = TrustedManagementComponent {
                git: git.into(),
                ..valid.clone()
            };
            assert!(ManagementAccess::new(&[candidate]).is_err(), "{git}");
        }
        for digest in ["latest".into(), "A".repeat(64), "a".repeat(63)] {
            let candidate = TrustedManagementComponent {
                digest,
                ..valid.clone()
            };
            assert!(ManagementAccess::new(&[candidate]).is_err());
        }
    }
}
