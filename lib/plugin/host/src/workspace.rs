use crate::{runtime, startup};
use dioxus::prelude::*;

#[cfg(any(feature = "web", feature = "desktop"))]
fn runtime_page(page: runtime::PageDefinition) -> az_dioxus_admin_shell::ApplicationRuntimePage {
    use az_dioxus_admin_shell::{ApplicationMenuGroup, ApplicationRuntimePage};

    ApplicationRuntimePage {
        id: page.id,
        label: page.label,
        icon: page.icon,
        scene_id: page.scene.id,
        scene_label: page.scene.label,
        menu_path: page
            .menu_path
            .into_iter()
            .map(|group| ApplicationMenuGroup {
                id: group.id,
                label: group.label,
                icon: group.icon,
            })
            .collect(),
        required_permission: page.required_permission,
        definition: serde_json::to_string(&page.body).unwrap_or_default(),
    }
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[dioxus::prelude::component]
pub fn Workspace(config: crate::composition::BrowserComposition) -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{ApplicationAccountItem, ApplicationUser, PluginApplication};
    use dioxus::prelude::*;

    let mut worker_open = use_signal(|| false);
    let mut worker_pair = use_signal(|| {
        web_sys::window()
            .and_then(|window| window.location().search().ok())
            .and_then(|search| {
                search.trim_start_matches('?').split('&').find_map(|part| {
                    part.strip_prefix("worker_pair=")
                        .filter(|code| {
                            code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                        .map(str::to_owned)
                })
            })
    });
    use_effect(move || {
        if worker_pair().is_some() {
            worker_open.set(true);
        }
    });
    let mut last_application = use_signal(|| None::<startup::LoadedApplication>);
    let mut preparations = use_signal(|| (String::new(), Vec::<String>::new()));
    let mut application = use_resource(move || {
        let previous = last_application.peek().clone();
        async move { startup::load(previous).await }
    });
    use_effect(move || {
        if let Some(Ok(value)) = application.read().as_ref() {
            last_application.set(Some(value.clone()));
        }
    });
    use_effect(move || {
        if application
            .read()
            .as_ref()
            .is_some_and(|result| result.as_ref().is_ok_and(|value| value.snapshot.is_none()))
        {
            spawn(async {
                let _ = document::eval("if (typeof caches !== 'undefined') { await Promise.all((await caches.keys()).filter(name => name.startsWith('aio-plugin-assets-v1-')).map(name => caches.delete(name))); } return true;").await;
            });
        }
    });
    use_future(move || async move {
        loop {
            let Ok(reason) = document::eval(include_str!("runtime/catalog_watch.js")).await else {
                break;
            };
            if application.finished() || reason.as_str() == Some("invalidated") {
                application.restart();
            }
        }
    });
    let result = application.read().as_ref().cloned();
    let result = match result {
        Some(Err(error)) => Some(
            last_application
                .read()
                .clone()
                .map(Ok)
                .unwrap_or(Err(error)),
        ),
        None => last_application.read().clone().map(Ok),
        result => result,
    };
    let Some(application_result) = result else {
        return rsx! { az_ui_components::admin::RequestState {} };
    };
    let snapshot = match application_result {
        Ok(startup::LoadedApplication {
            snapshot: Some(snapshot),
            ..
        }) => snapshot,
        Ok(startup::LoadedApplication { snapshot: None, .. }) => {
            return rsx! { az_ui_components::appearance::AppearanceScope { user_key: String::new(), {(config.login)()} } };
        }
        Err(error) => {
            return rsx! {
                az_ui_components::admin::RequestState { error, on_retry: move |_| application.restart() }
            };
        }
    };
    let catalog = snapshot.catalog;
    let preload = serde_json::json!({
        "session_context": catalog.session_context,
        "context": catalog.context,
        "pages": catalog.pages.iter().filter(|page| {
            !catalog.hidden_pages.contains(&page.id)
                && !catalog.plugin_settings.iter().any(|item| item.page_id == page.id)
                && matches!(page.body, runtime::PageBody::Frontend { .. })
                && page.required_permission.as_deref().is_none_or(|permission| snapshot.permissions.iter().any(|item| item == permission))
        }).map(|page| serde_json::json!({ "id": page.id, "version": catalog.page_versions.get(&page.id) })).collect::<Vec<_>>()
    }).to_string();
    let preparation_context = preload.clone();
    let prepared_pages = if preparations.read().0 == preload {
        preparations.read().1.clone()
    } else {
        Vec::new()
    };
    let mut static_plugins = config.clone();
    static_plugins.pages.retain(|page| {
        page.required_permission
            .is_none_or(|permission| snapshot.permissions.iter().any(|item| item == permission))
    });
    static_plugins.account_items.retain(|item| {
        item.required_permission
            .as_deref()
            .is_none_or(|permission| snapshot.permissions.iter().any(|value| value == permission))
    });
    let mut account_items = static_plugins.account_items;
    account_items.push(ApplicationAccountItem {
        id: "worker-devices".into(),
        label: "我的设备".into(),
        icon: Some("monitor".into()),
        page_id: None,
        required_permission: None,
        destructive: false,
    });
    account_items.extend(
        catalog
            .account_items
            .into_iter()
            .filter(|item| {
                !catalog.hidden_pages.contains(&item.page_id)
                    && item
                        .required_permission
                        .as_deref()
                        .is_none_or(|permission| {
                            snapshot
                                .permissions
                                .iter()
                                .any(|candidate| candidate == permission)
                        })
            })
            .map(|item| ApplicationAccountItem {
                id: item.id,
                label: item.label,
                icon: item.icon,
                page_id: Some(item.page_id),
                required_permission: item.required_permission,
                destructive: false,
            }),
    );
    let runtime_pages = catalog
        .pages
        .into_iter()
        .filter(|page| {
            !catalog.hidden_pages.contains(&page.id)
                && !catalog
                    .plugin_settings
                    .iter()
                    .any(|item| item.page_id == page.id)
        })
        .map(runtime_page)
        .collect::<Vec<_>>();
    rsx! {
        runtime::frontend_preload::FrontendPreload {
            key: "{preload}", config: preload.clone(),
            on_prepare: move |id: String| {
                let mut value = preparations.write();
                if value.0 != preparation_context { *value = (preparation_context.clone(), Vec::new()); }
                if !value.1.contains(&id) { value.1.push(id); }
            },
        }
        for context in [catalog.session_context] {
          az_ui_components::appearance::AppearanceScope { key: "{context}", user_key: catalog.user.handle.clone(),
          if worker_open() {
              crate::generated::worker::view::WorkerPanel { pairing: worker_pair, on_close: move |_| { worker_open.set(false); worker_pair.set(None); } }
          }
          runtime::settings::PluginSettingsHost { pages: catalog.plugin_settings.clone(), versions: catalog.page_versions.clone(), context: catalog.context.clone(),
          PluginApplication {
            application_label: config.label.clone(),
            pages: static_plugins.pages.clone(),
            account_items: account_items.clone(),
            topbar_items: config.topbar_items.clone(),
            runtime_pages: runtime_pages.clone(),
            runtime_page_versions: catalog.page_versions.clone(),
            prepared_pages: prepared_pages.clone(),
            workspace_id: catalog.tenant.id.clone(),
            workspace_context: catalog.context.clone(),
            render_runtime_page: runtime::client::render_page,
            on_account_action: move |action: String| {
                if action == "worker-devices" { worker_open.set(true); } else { (config.account_action)(action); }
            },
            user: ApplicationUser {
                label: catalog.user.label.clone(),
                handle: catalog.user.handle.clone(),
                initials: catalog.user.initials.clone(),
            },
          }
          }
          }
        }
    }
}

#[cfg(all(test, any(feature = "web", feature = "desktop")))]
mod tests {
    use az_plugin_manifest::{PageBody, PageDefinition, SceneDefinition};

    use super::runtime_page;

    #[test]
    fn runtime_page_preserves_plugin_scene() {
        let page = PageDefinition {
            id: "documentation.home".into(),
            label: "首页".into(),
            icon: None,
            scene: SceneDefinition {
                id: "documentation-agent".into(),
                label: "资料员服务平台".into(),
            },
            menu_path: Vec::new(),
            required_permission: None,
            body: PageBody::Text {
                title: "首页".into(),
                content: "ok".into(),
            },
        };
        let converted = runtime_page(page);
        assert_eq!(converted.scene_id, "documentation-agent");
        assert_eq!(converted.scene_label, "资料员服务平台");
    }
}
