//! Rust 整包组件使用的客户契约，直接从平台唯一 WIT 定义生成绑定。
//!
//! 组件实现 `Guest` 并调用 `export!(Component)`，无需复制 WIT 或携带宿主执行器。

// WIT 展平后的 ABI 参数由生成器决定，不适用手写 Rust 函数的参数数量限制。
#[allow(clippy::too_many_arguments)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../contract/wit",
        world: "plugin",
        pub_export_macro: true,
        default_bindings_module: "az_plugin_guest::bindings",
    });
}

pub use bindings::*;
