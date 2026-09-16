use super::PluginSettingsPage;
use az_dioxus_admin_shell::{ApplicationSettings, ApplicationSettingsGroup};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
use std::collections::BTreeMap;

/// 给设置中心提供已授权分组和挂载槽，同时处理市场详情的快捷入口。
#[component]
pub(crate) fn PluginSettingsHost(
    pages: Vec<PluginSettingsPage>,
    versions: BTreeMap<String, String>,
    context: String,
    children: Element,
) -> Element {
    let mut groups = use_signal(Vec::new);
    let selected_group = use_signal(|| None::<String>);
    use_effect(use_reactive((&pages,), move |(pages,)| {
        groups.set(
            pages
                .iter()
                .map(|page| ApplicationSettingsGroup {
                    page_id: page.page_id.clone(),
                    title: page.label.clone(),
                })
                .collect(),
        );
    }));
    let setting_pages = pages.clone();
    let setting_versions = versions.clone();
    let setting_context = context.clone();
    let render = use_callback(move |page_id: String| {
        let Some(page) = setting_pages.iter().find(|page| page.page_id == page_id) else {
            return rsx! { p { role: "alert", "此插件设置已不可用。" } };
        };
        let version = setting_versions.get(&page_id).cloned().unwrap_or_default();
        rsx! {
            super::frontend::RuntimeFrontend {
                key: "{setting_context}:{page_id}:{version}",
                page_id,
                label: format!("{}设置", page.label),
            }
        }
    });
    use_context_provider(|| ApplicationSettings {
        selected: selected_group,
        groups: groups.into(),
        render,
    });
    let mut selected = use_signal(|| None::<String>);
    let mut listener = use_signal(|| None::<document::Eval>);
    use_future(move || async move {
        let mut events = document::eval(
            r#"
            const handle = event => { if (typeof event.detail?.pageId === 'string') dioxus.send(event.detail.pageId); };
            window.addEventListener('aio:plugin-settings', handle);
            await dioxus.recv();
            window.removeEventListener('aio:plugin-settings', handle);
        "#,
        );
        listener.set(Some(events));
        while let Ok(page) = events.recv::<String>().await {
            selected.set(Some(page));
        }
    });
    use_drop(move || {
        if let Some(listener) = listener() {
            let _ = listener.send(());
        }
    });
    let current = selected().and_then(|id| pages.iter().find(|page| page.page_id == id).cloned());
    rsx! {
        {children}
        if let Some(page) = current {
            Dialog { key: "{context}:{page.page_id}:{versions.get(&page.page_id).cloned().unwrap_or_default()}", class: "dx-plugin-settings", open: true, on_open_change: move |open: bool| if !open {selected.set(None)},
                div { class: "dx-plugin-settings__heading",
                    div {
                        DialogTitle { "{page.label}设置" }
                        small { class: "workbench-settings__source", "来自插件（{page.label}）" }
                    }
                    Button { variant: ButtonVariant::Ghost, onclick: move |_| selected.set(None), "关闭" }
                }
                super::frontend::RuntimeFrontend { page_id: page.page_id, label: format!("{}设置",page.label) }
            }
        }
    }
}
