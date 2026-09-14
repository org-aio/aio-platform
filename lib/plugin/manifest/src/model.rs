use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RepositoryManifest {
    pub plugin: PluginManifest,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<RepositoryDependency>,
    #[serde(default)]
    pub marketplace: Option<MarketplaceManifest>,
    #[serde(default)]
    pub client: Option<RepositoryPackage>,
    #[serde(default)]
    pub server: Option<RepositoryPackage>,
    #[serde(default)]
    pub runtime: Option<RuntimeManifest>,
    #[serde(default)]
    pub frontend: Option<FrontendManifest>,
    #[serde(default)]
    pub capabilities: CapabilityManifest,
    #[serde(default)]
    pub subplugins: Vec<SubpluginManifest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RepositoryDependency {
    pub git: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MarketplaceManifest {
    pub title: String,
    pub summary: String,
    pub license: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RepositoryPackage {
    #[serde(default)]
    pub runtime: PluginRuntime,
    #[serde(default = "current_directory")]
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub kind: PluginRuntime,
    pub artifact: String,
    #[serde(default)]
    pub host_version: Option<String>,
    #[serde(default)]
    pub container_image: Option<String>,
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default)]
    pub health_check: Option<String>,
    #[serde(default)]
    pub shutdown_timeout_seconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FrontendManifest {
    pub path: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum PluginRuntime {
    #[default]
    RustSource,
    PageDefinition,
    WasmComponent,
    Process,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CapabilityManifest {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub database: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SubpluginManifest {
    pub id: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub account_actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PageDefinition {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub scene: SceneDefinition,
    #[serde(default)]
    pub menu_path: Vec<MenuGroupDefinition>,
    #[serde(default)]
    pub required_permission: Option<String>,
    pub body: PageBody,
}

/// 页面在当前场景菜单树中的一个目录节点。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MenuGroupDefinition {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SceneDefinition {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageBody {
    Frontend {
        entry: String,
    },
    Counter {
        title: String,
        button: String,
    },
    Text {
        title: String,
        content: String,
    },
    Actions {
        title: String,
        content: String,
        #[serde(default)]
        state: BTreeMap<String, Value>,
        actions: Vec<PageActionDefinition>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PageActionDefinition {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PageActionResult {
    pub body: PageBody,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginRequest {
    ServiceRequest {
        method: String,
        path: String,
        query: Option<String>,
        body: String,
        tenant_id: String,
        user_id: String,
    },
    PageAction {
        page_id: String,
        action_id: String,
        tenant_id: String,
        user_id: String,
        body: PageBody,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ComponentResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

fn current_directory() -> String {
    ".".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_typed_plugin_requests() -> Result<(), serde_json::Error> {
        let request = PluginRequest::PageAction {
            page_id: "counter".to_owned(),
            action_id: "increment".to_owned(),
            tenant_id: "tenant-a".to_owned(),
            user_id: "user-a".to_owned(),
            body: PageBody::Actions {
                title: "Counter".to_owned(),
                content: "1".to_owned(),
                state: [("count".to_owned(), serde_json::json!(1))]
                    .into_iter()
                    .collect(),
                actions: vec![PageActionDefinition {
                    id: "increment".to_owned(),
                    label: "+1".to_owned(),
                }],
            },
        };
        let json = serde_json::to_string(&request)?;

        assert_eq!(serde_json::from_str::<PluginRequest>(&json)?, request);
        assert!(json.contains("\"kind\":\"page_action\""));
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_request_and_page_fields() {
        assert!(
            serde_json::from_str::<PluginRequest>(
                r#"{"method":"GET","path":"/echo","body":"","tenant_id":"t","user_id":"u"}"#,
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<PageBody>(
                r#"{"kind":"text","title":"Hello","content":"World","html":"<b>World</b>"}"#,
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ComponentResponse>(
                r#"{"status":200,"content_type":"text/plain","body":"ok","headers":{}}"#,
            )
            .is_err()
        );
    }
}
