use dioxus::prelude::*;
use rudi::Context as RudiContext;

use crate::{
    BuiltInPageContext, BuiltInPageIndex, CompiledPageRenderer, ConventionPageContext,
    EndpointWorkbench, MenuRowActions, ProgramImage, SymbolId, browser_http::get_api,
};

#[component]
pub fn EndpointPage(page_id: String) -> Element {
    let image = use_resource(move || async move {
        get_api::<ProgramImage>("", "/api/runtime/program/image").await
    });
    let Some(result) = image.read().as_ref().cloned() else {
        return state("正在加载接口页面", false);
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return state(&error, true),
    };
    let page_id = match SymbolId::parse(&page_id) {
        Ok(page_id) => page_id,
        Err(error) => return state(&error.to_string(), true),
    };
    let Some(page) = image.pages.get(&page_id).cloned() else {
        return state("接口页面编译产物不存在", true);
    };
    rsx! {
        EndpointWorkbench {
            context: ConventionPageContext {
                api_base_url: String::new(),
                route: String::new(),
                page,
            }
        }
    }
}

#[component]
pub fn RuntimePage(page_id: String) -> Element {
    let image = use_resource(move || async move {
        get_api::<ProgramImage>("", "/api/runtime/program/image").await
    });
    let Some(result) = image.read().as_ref().cloned() else {
        return state("正在加载页面", false);
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return state(&error, true),
    };
    let page_id = match SymbolId::parse(&page_id) {
        Ok(page_id) => page_id,
        Err(error) => return state(&error.to_string(), true),
    };
    let Some(page) = image.pages.get(&page_id).cloned() else {
        return state("页面编译产物不存在", true);
    };
    let provider_key = match &page.renderer {
        CompiledPageRenderer::MenuTree { provider_key }
        | CompiledPageRenderer::TreeTable { provider_key, .. }
        | CompiledPageRenderer::CrudTable { provider_key, .. } => provider_key.clone(),
        CompiledPageRenderer::ConventionFile { .. } | CompiledPageRenderer::Extension { .. } => {
            return state("AIO 运行时扩展不能渲染当前页面", true);
        }
    };
    let mut context = RudiContext::auto_register();
    let providers = match BuiltInPageIndex::from_context(&mut context) {
        Ok(providers) => providers,
        Err(error) => return state(&error.to_string(), true),
    };
    let row_actions = row_actions_for_page(&image.menus, page_id).unwrap_or_default();
    providers
        .render(
            &provider_key,
            BuiltInPageContext {
                api_base_url: String::new(),
                image,
                page,
                row_actions,
            },
        )
        .unwrap_or_else(|| state("AIO 内置页面 Provider 未注册", true))
}

fn row_actions_for_page(
    menus: &[crate::MenuDefinition],
    page_id: SymbolId,
) -> Option<MenuRowActions> {
    menus.iter().find_map(|menu| {
        if menu.page_id == Some(page_id) {
            return Some(menu.row_actions.clone());
        }
        row_actions_for_page(&menu.children, page_id)
    })
}

fn state(message: &str, error: bool) -> Element {
    rsx! {
        div {
            class: if error { "aio-runtime-error" } else { "aio-runtime-table-state" },
            role: if error { "alert" } else { "status" },
            "{message}"
        }
    }
}
