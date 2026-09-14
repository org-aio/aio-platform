use az_dioxus_admin_shell::{ApplicationAccountItem, ApplicationPage};
use dioxus::prelude::*;

/// 产品装配静态贡献；沙箱传入空组合，只显示运行集合中的插件。
#[derive(Clone, PartialEq)]
pub struct BrowserComposition {
    pub label: String,
    pub pages: Vec<ApplicationPage>,
    pub account_items: Vec<ApplicationAccountItem>,
    pub login: fn() -> Element,
    pub account_action: fn(String),
}
