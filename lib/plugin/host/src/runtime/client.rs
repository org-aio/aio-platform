use az_dioxus_admin_shell::ApplicationRuntimePage;
use az_plugin_manifest::PageActionDefinition;
use az_ui_components::button::{Button, ButtonVariant};
use dioxus::prelude::*;

use super::{PageActionRequest, PageActionResult, PageBody, RuntimeResponse};

pub fn render_page(page: ApplicationRuntimePage) -> Element {
    let page_key = page.id.clone();
    rsx! { RuntimePage { key: "{page_key}", page } }
}

#[component]
fn RuntimePage(page: ApplicationRuntimePage) -> Element {
    let definition = page.definition.clone();
    let body = use_signal(move || {
        serde_json::from_str::<PageBody>(&definition).map_err(|error| error.to_string())
    });
    match body() {
        Ok(PageBody::Frontend { .. }) => rsx! {
            super::frontend::RuntimeFrontend { page_id: page.id, label: page.label }
        },
        Ok(PageBody::Counter { title, button }) => rsx! {
            RuntimeCounter { title, button }
        },
        Ok(PageBody::Text { title, content }) => rsx! {
            section { h2 { "{title}" } p { "{content}" } }
        },
        Ok(PageBody::Actions {
            title,
            content,
            actions,
            ..
        }) => rsx! {
            RuntimeActions {
                page_id: page.id,
                title,
                content,
                actions,
                body,
            }
        },
        Err(error) => rsx! { p { role: "alert", "页面定义无效: {error}" } },
    }
}

#[component]
fn RuntimeCounter(title: String, button: String) -> Element {
    let mut count = use_signal(|| 0_u64);
    rsx! {
        section {
            h2 { "{title}" }
            p { "计数：{count}" }
            Button {
                r#type: "button",
                variant: ButtonVariant::Outline,
                aria_label: button.clone(),
                onclick: move |_| count.with_mut(|value| *value = value.saturating_add(1)),
                "{button}"
            }
        }
    }
}

#[component]
fn RuntimeActions(
    page_id: String,
    title: String,
    content: String,
    actions: Vec<PageActionDefinition>,
    body: Signal<Result<PageBody, String>>,
) -> Element {
    let pending = use_signal(|| None::<String>);
    let error = use_signal(|| None::<String>);
    rsx! {
        section {
            h2 { "{title}" }
            p { "{content}" }
            div { class: "flex gap-2",
                for action in actions {
                    RuntimeActionButton {
                        key: "{action.id}",
                        page_id: page_id.clone(),
                        action,
                        body,
                        pending,
                        error,
                    }
                }
            }
            if let Some(message) = error() {
                p { role: "alert", "{message}" }
            }
        }
    }
}

#[component]
fn RuntimeActionButton(
    page_id: String,
    action: PageActionDefinition,
    mut body: Signal<Result<PageBody, String>>,
    mut pending: Signal<Option<String>>,
    mut error: Signal<Option<String>>,
) -> Element {
    let action_id = action.id.clone();
    let pending_action = action.id.clone();
    rsx! {
        Button {
            r#type: "button",
            variant: ButtonVariant::Outline,
            aria_label: action.label.clone(),
            disabled: pending().is_some(),
            onclick: move |_| {
                let request = PageActionRequest {
                    page_id: page_id.clone(),
                    action_id: action_id.clone(),
                };
                pending.set(Some(pending_action.clone()));
                error.set(None);
                spawn(async move {
                    match invoke_page_action(request).await {
                        Ok(result) => body.set(Ok(result.body)),
                        Err(message) => error.set(Some(message)),
                    }
                    pending.set(None);
                });
            },
            "{action.label}"
        }
    }
}

async fn invoke_page_action(request: PageActionRequest) -> Result<PageActionResult, String> {
    let response = gloo_net::http::Request::post("/api/runtime/pages/action")
        .json(&request)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response.text().await.unwrap_or_default());
    }
    response
        .json::<RuntimeResponse<PageActionResult>>()
        .await
        .map(|response| response.data)
        .map_err(|error| error.to_string())
}

pub fn reload() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}
