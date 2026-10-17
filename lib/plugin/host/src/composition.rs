use az_dioxus_admin_shell::{ApplicationAccountItem, ApplicationPage, ApplicationTopbarItem};
use dioxus::prelude::*;

/// 产品装配静态贡献；沙箱传入空组合，只显示运行集合中的插件。
#[derive(Clone)]
pub struct BrowserComposition {
    pub label: String,
    pub pages: Vec<ApplicationPage>,
    pub account_items: Vec<ApplicationAccountItem>,
    pub topbar_items: Vec<ApplicationTopbarItem>,
    pub login: fn() -> Element,
    pub account_action: fn(String),
}

// 回调地址只用于组件属性比较，不承担插件身份；变化时允许 Dioxus 重新渲染。
impl PartialEq for BrowserComposition {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
            && self.pages == other.pages
            && self.account_items == other.account_items
            && self.topbar_items == other.topbar_items
            && std::ptr::fn_addr_eq(self.login, other.login)
            && std::ptr::fn_addr_eq(self.account_action, other.account_action)
    }
}
