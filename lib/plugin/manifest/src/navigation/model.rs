use serde::{Deserialize, Serialize};

use crate::{PageBody, PageDefinition};

/// 导航编写文档与已展开的页面协议共用一个接收入口。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum PageDocument {
    Scene(SceneTree),
    Scenes(Vec<SceneTree>),
    Pages(Vec<PageDefinition>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SceneTree {
    pub id: String,
    pub label: String,
    pub children: Vec<MenuNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum MenuNode {
    Group(MenuBranch),
    Page(MenuPage),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MenuBranch {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub children: Vec<MenuNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MenuPage {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub required_permission: Option<String>,
    pub body: PageBody,
}
