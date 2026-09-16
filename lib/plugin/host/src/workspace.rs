use crate::{runtime, startup};
use dioxus::prelude::*;

#[cfg(any(feature = "web", feature = "desktop"))]
#[dioxus::prelude::component]
pub fn Workspace(config: crate::composition::BrowserComposition) -> dioxus::prelude::Element {
    use az_dioxus_admin_shell::{
        ApplicationAccountItem, ApplicationMenuGroup, ApplicationRuntimePage, ApplicationUser,
        PluginApplication,
    };
    use dioxus::prelude::*;

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
        .map(|page| ApplicationRuntimePage {
            id: page.id,
            label: page.label,
            icon: page.icon,
            scene_id: "workspace".into(),
            scene_label: "工作空间".into(),
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
        })
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
          runtime::settings::PluginSettingsHost { pages: catalog.plugin_settings.clone(), versions: catalog.page_versions.clone(), context: catalog.context.clone(),
          PluginApplication {
            application_label: config.label.clone(),
            pages: static_plugins.pages.clone(),
            account_items: account_items.clone(),
            runtime_pages: runtime_pages.clone(),
            runtime_page_versions: catalog.page_versions.clone(),
            prepared_pages: prepared_pages.clone(),
            workspace_id: catalog.tenant.id.clone(),
            workspace_context: catalog.context.clone(),
            render_runtime_page: runtime::client::render_page,
            on_account_action: config.account_action,
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
