#![forbid(unsafe_code)]

use crate::{CompiledRoute, MenuDefinition, SymbolId};
use serde::{Deserialize, Serialize};

/// Shell、登录恢复和 Studio 的不可配置母机入口。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeEntry {
    pub route: String,
    pub title: String,
    pub kind: NativeEntryKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeEntryKind {
    Studio,
    Login,
    Recovery,
}

/// 浏览器启动所需的活动应用轻量快照。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishedApplication {
    pub application_id: String,
    pub program_id: SymbolId,
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
    pub native_entries: Vec<NativeEntry>,
    pub applications: Vec<PublishedApplication>,
}

impl Default for WorkbenchBootstrap {
    fn default() -> Self {
        Self {
            api_base_url: String::new(),
            default_route: "/studio".to_owned(),
            native_entries: vec![NativeEntry {
                route: "/studio".to_owned(),
                title: "Studio".to_owned(),
                kind: NativeEntryKind::Studio,
            }],
            applications: Vec::new(),
        }
    }
}

impl WorkbenchBootstrap {
    #[must_use]
    pub fn route(&self, path: &str) -> Option<(&PublishedApplication, &CompiledRoute)> {
        self.applications.iter().find_map(|application| {
            application
                .routes
                .iter()
                .find(|route| route.path == path)
                .map(|route| (application, route))
        })
    }
}
