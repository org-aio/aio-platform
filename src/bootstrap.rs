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

impl PublishedProgram {
    #[must_use]
    pub fn default_route(&self) -> Option<String> {
        self.menus
            .iter()
            .find_map(|menu| self.first_menu_route(menu))
    }

    #[must_use]
    pub fn first_menu_route(&self, menu: &MenuDefinition) -> Option<String> {
        menu.page_id
            .and_then(|page_id| {
                self.routes
                    .iter()
                    .find(|route| route.page_id == page_id)
                    .map(|route| route.path.clone())
            })
            .or_else(|| {
                menu.children
                    .iter()
                    .find_map(|child| self.first_menu_route(child))
            })
    }
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

/// Studio 自身提供工作台编辑能力，不依赖业务插件决定管理模式。
#[cfg(not(target_arch = "wasm32"))]
#[rudi::Singleton(name = module_path!())]
pub fn admin_workbench_state() -> AdminWorkbenchState {
    AdminWorkbenchState {
        can_add_scene: true,
        can_add_menu: true,
        can_edit_page: true,
    }
}

/// 从 Studio 的 Rudi 注册中解析工作台能力。
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_admin_workbench_state(context: &mut rudi::Context) -> Option<AdminWorkbenchState> {
    context.resolve_option_with_name::<AdminWorkbenchState>(module_path!())
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

    fn menu(page_id: Option<SymbolId>, children: Vec<MenuDefinition>) -> MenuDefinition {
        MenuDefinition {
            id: SymbolId::new(),
            name: "menu".to_owned(),
            title: "菜单".to_owned(),
            state: crate::DefinitionState::Known,
            icon: None,
            page_id,
            enabled: true,
            children,
            required_permissions: Vec::new(),
            row_actions: crate::MenuRowActions::default(),
        }
    }

    #[test]
    fn default_bootstrap_does_not_publish_studio_as_sidebar_entry() {
        let bootstrap = WorkbenchBootstrap::default();

        assert!(bootstrap.native_entries.is_empty());
        assert_eq!(bootstrap.default_route, "/studio");
    }

    #[test]
    fn studio_registers_admin_workbench_state() {
        crate::enable();
        let mut context = rudi::Context::auto_register();

        assert_eq!(
            resolve_admin_workbench_state(&mut context),
            Some(AdminWorkbenchState {
                can_add_scene: true,
                can_add_menu: true,
                can_edit_page: true,
            })
        );
    }

    #[test]
    fn program_default_route_comes_from_the_published_menu_tree() {
        let hidden_page_id = SymbolId::new();
        let visible_page_id = SymbolId::new();
        let program = PublishedProgram {
            id: SymbolId::new(),
            name: "program".to_owned(),
            title: "程序".to_owned(),
            revision_id: "revision".to_owned(),
            content_hash: "hash".to_owned(),
            menus: vec![menu(None, vec![menu(Some(visible_page_id), Vec::new())])],
            routes: vec![
                CompiledRoute {
                    id: SymbolId::new(),
                    name: "hidden".to_owned(),
                    path: "/hidden".to_owned(),
                    page_id: hidden_page_id,
                    required_permissions: Vec::new(),
                },
                CompiledRoute {
                    id: SymbolId::new(),
                    name: "visible".to_owned(),
                    path: "/visible".to_owned(),
                    page_id: visible_page_id,
                    required_permissions: Vec::new(),
                },
            ],
        };

        assert_eq!(program.default_route().as_deref(), Some("/visible"));
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
