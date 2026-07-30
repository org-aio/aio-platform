#![forbid(unsafe_code)]

use crate::{CompiledRoute, MenuDefinition, SymbolId};
use serde::{Deserialize, Serialize};

/// Shell 登录与恢复使用的不可配置管理入口。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeEntry {
    pub route: String,
    pub title: String,
    pub kind: NativeEntryKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeEntryKind {
    Login,
    Recovery,
}

/// 浏览器启动所需的唯一活动程序轻量快照。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishedProgram {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    pub revision_id: String,
    pub content_hash: String,
    pub menus: Vec<MenuDefinition>,
    pub routes: Vec<CompiledRoute>,
}

/// 服务端与 WASM 客户端共用且不包含渲染实现的启动协议。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchBootstrap {
    pub api_base_url: String,
    pub default_route: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<AdminWorkbenchState>,
    pub native_entries: Vec<NativeEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<PublishedProgram>,
}

/// 服务端解析 Rudi 状态后下发的管理模式能力。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminWorkbenchState {
    pub can_add_scene: bool,
    pub can_add_menu: bool,
    pub can_edit_page: bool,
}

impl Default for WorkbenchBootstrap {
    fn default() -> Self {
        Self {
            api_base_url: String::new(),
            default_route: "/studio".to_owned(),
            admin: None,
            native_entries: Vec::new(),
            program: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bootstrap_does_not_publish_studio_as_sidebar_entry() {
        let bootstrap = WorkbenchBootstrap::default();

        assert!(bootstrap.native_entries.is_empty());
        assert_eq!(bootstrap.default_route, "/studio");
    }
}

impl WorkbenchBootstrap {
    #[must_use]
    pub fn route(&self, path: &str) -> Option<(&PublishedProgram, &CompiledRoute)> {
        let program = self.program.as_ref()?;
        program
            .routes
            .iter()
            .find(|route| route.path == path)
            .map(|route| (program, route))
    }
}
