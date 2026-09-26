use dioxus::prelude::*;
use serde::Deserialize;

use super::RuntimeResponse;
use az_ui_components::button::{Button, ButtonVariant};

const FRONTEND_LOADING_STYLE: &str = "position:absolute;inset:0;z-index:1;display:flex;align-items:center;justify-content:center;gap:10px;background:var(--background);color:var(--muted-foreground);font-size:14px";
const FRONTEND_SPINNER_STYLE: &str = "width:18px;height:18px;border:2px solid var(--border);border-top-color:var(--primary);border-radius:50%;animation:spin .8s linear infinite";

#[derive(Clone, Deserialize, PartialEq)]
struct FrontendMount {
    #[serde(default)]
    development: bool,
    #[serde(default)]
    abi: Option<u32>,
    token: String,
    src: String,
    revision: String,
    generation: String,
    session_context: String,
    context: String,
    assets: std::collections::BTreeMap<String, String>,
}

#[component]
pub(super) fn RuntimeFrontend(page_id: String, label: String) -> Element {
    let mut invalid = use_signal(|| None::<String>);
    let requested_page = page_id.clone();
    let mut mount = use_resource(move || {
        let page_id = requested_page.clone();
        async move {
            let response = gloo_net::http::Request::post("/api/runtime/frontend/mount")
                .json(&serde_json::json!({ "page_id": page_id }))
                .map_err(|error| error.to_string())?
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.ok() {
                return Err(response
                    .text()
                    .await
                    .unwrap_or_else(|_| "挂载前端失败".to_owned()));
            }
            response
                .json::<RuntimeResponse<FrontendMount>>()
                .await
                .map(|response| response.data)
                .map_err(|error| error.to_string())
        }
    });
    let failure = invalid().or_else(|| {
        mount
            .read()
            .as_ref()
            .and_then(|value| value.as_ref().err().cloned())
    });
    if let Some(message) = failure {
        return rsx! {
            p { role: "alert", "加载插件页面失败: {message}" }
            Button {
                variant: ButtonVariant::Outline,
                onclick: move |_| { mount.clear(); invalid.set(None); mount.restart(); },
                "重新打开"
            }
        };
    }
    match mount.read().as_ref() {
        Some(Ok(mount)) => {
            rsx! { MountedFrontend { key: "{mount.token}", mount: mount.clone(), page_id, label, on_error: move |error| invalid.set(Some(error)) } }
        }
        Some(Err(error)) => rsx! { p { role: "alert", "加载插件页面失败: {error}" } },
        None => rsx! { p { role: "status", "正在加载插件页面" } },
    }
}

#[component]
fn MountedFrontend(
    mount: FrontendMount,
    page_id: String,
    label: String,
    on_error: Callback<String>,
) -> Element {
    let mut bridge = use_signal(|| None::<document::Eval>);
    let frame_ready = use_signal(|| false);
    let frame_id = format!("aio-frontend-{}", mount.token);
    let config = serde_json::json!({ "development": mount.development, "abi": mount.abi, "id": frame_id, "page_id": page_id, "token": mount.token, "src": mount.src, "revision": mount.revision, "generation": mount.generation, "session_context": mount.session_context, "context": mount.context, "assets": mount.assets });
    use_drop(move || {
        if let Some(bridge) = bridge() {
            let _ = bridge.send(serde_json::json!({ "dispose": true }));
        }
    });
    rsx! {
        div { class: "application-frontend-shell", style: "position:relative;flex:1 1 auto;width:100%;height:100%;min-width:0;min-height:0",
            if !frame_ready() {
                div { class: "application-frontend-loading", style: FRONTEND_LOADING_STYLE, role: "status", aria_live: "polite", aria_busy: "true",
                    span { style: FRONTEND_SPINNER_STYLE, aria_hidden: "true" }
                    span { "正在加载页面" }
                }
            }
            iframe {
                id: frame_id,
                title: label,
                class: "application-frontend",
                "sandbox": "allow-scripts allow-forms",
                allow: "fullscreen; clipboard-write",
                referrerpolicy: "no-referrer",
                onmounted: move |_| {
                    if bridge().is_none() {
                        let script = if mount.abi == Some(2) { concat!(include_str!("frontend_lifecycle.js"), "\n", include_str!("frontend_cache.js"), "\n", include_str!("frontend_assets.js"), "\n", include_str!("frontend_component.js")) } else { concat!(include_str!("frontend_lifecycle.js"), "\n", include_str!("frontend_cache.js"), "\n", include_str!("frontend_host.js")) };
                        let mut evaluator = document::eval(script);
                        match evaluator.send(config.clone()) {
                            Ok(()) => {
                                bridge.set(Some(evaluator));
                                let mut frame_ready = frame_ready;
                                spawn(async move {
                                    let Ok(message) = evaluator.recv::<serde_json::Value>().await else {
                                        return;
                                    };
                                    if let Some(error) = message.get("error").and_then(|value| value.as_str()) {
                                        on_error.call(error.to_owned());
                                    } else if message.get("ready").and_then(|value| value.as_bool()) == Some(true) {
                                        frame_ready.set(true);
                                    }
                                });
                            },
                            Err(cause) => on_error.call(format!("启动插件通信桥失败: {cause}")),
                        }
                    }
                },
            }
        }
    }
}
