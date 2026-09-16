use super::PluginSettingsPage;
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
use std::collections::BTreeMap;

/// 市场与设置中心只发出页面选择，挂载和授权统一由宿主处理。
#[component]
pub(crate) fn PluginSettingsHost(
    pages: Vec<PluginSettingsPage>,
    versions: BTreeMap<String, String>,
    context: String,
) -> Element {
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
        if let Some(page) = current {
            Dialog { key: "{context}:{page.page_id}:{versions.get(&page.page_id).cloned().unwrap_or_default()}", class: "dx-plugin-settings", open: true, on_open_change: move |open: bool| if !open {selected.set(None)},
                div { class: "dx-plugin-settings__heading",
                    DialogTitle { "{page.label} · 插件设置" }
                    Button { variant: ButtonVariant::Ghost, onclick: move |_| selected.set(None), "关闭" }
                }
                super::frontend::RuntimeFrontend { page_id: page.page_id, label: format!("{}设置",page.label) }
            }
        }
    }
}
