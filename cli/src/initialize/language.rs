use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PluginLanguage {
    #[default]
    Rust,
    Kotlin,
    TypeScript,
}

impl PluginLanguage {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "rust" => Ok(Self::Rust),
            "kotlin" => Ok(Self::Kotlin),
            "typescript" => Ok(Self::TypeScript),
            _ => bail!("插件语言必须是 rust、kotlin 或 typescript: {value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginTemplate {
    Cli,
    Fullstack(PluginLanguage),
    WebFullstack(WebFramework),
    Rust,
    KotlinPages,
    KotlinComponent,
    KotlinService,
    TypeScriptPages,
    TypeScriptComponent,
    TypeScriptService,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebFramework {
    Nuxt,
    Next,
}

impl WebFramework {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "nuxt" => Ok(Self::Nuxt),
            "next" => Ok(Self::Next),
            _ => bail!("全栈框架必须是 nuxt 或 next: {value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginRuntimeOverride {
    PageDefinition,
    WasmComponent,
    Process,
}

impl PluginTemplate {
    pub fn resolve(
        language: PluginLanguage,
        runtime: Option<PluginRuntimeOverride>,
    ) -> Result<Self> {
        match (language, runtime) {
            (language, None) => Ok(Self::Fullstack(language)),
            (PluginLanguage::Rust, Some(_)) => {
                bail!("Rust 源码插件无需 --runtime；实现插件 trait 后由 Dill 按 TypeId 自动聚合")
            }
            (PluginLanguage::Kotlin, Some(PluginRuntimeOverride::Process)) => {
                Ok(Self::KotlinService)
            }
            (PluginLanguage::Kotlin, Some(PluginRuntimeOverride::PageDefinition)) => {
                Ok(Self::KotlinPages)
            }
            (PluginLanguage::Kotlin, Some(PluginRuntimeOverride::WasmComponent)) => {
                Ok(Self::KotlinComponent)
            }
            (PluginLanguage::TypeScript, Some(PluginRuntimeOverride::WasmComponent)) => {
                Ok(Self::TypeScriptComponent)
            }
            (PluginLanguage::TypeScript, Some(PluginRuntimeOverride::PageDefinition)) => {
                Ok(Self::TypeScriptPages)
            }
            (PluginLanguage::TypeScript, Some(PluginRuntimeOverride::Process)) => {
                Ok(Self::TypeScriptService)
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cli => "TypeScript CLI（npm 与插件市场自动发布）",
            Self::Fullstack(PluginLanguage::Rust) => "Rust 全栈插件",
            Self::Fullstack(PluginLanguage::Kotlin) => "Kotlin 全栈插件",
            Self::Fullstack(PluginLanguage::TypeScript) => "TypeScript 全栈插件",
            Self::WebFullstack(WebFramework::Nuxt) => "Nuxt 全栈插件",
            Self::WebFullstack(WebFramework::Next) => "Next.js 全栈插件",
            Self::Rust => "Rust 源码插件（Dill/TypeId 自动聚合）",
            Self::KotlinPages => "Kotlin 静态页面",
            Self::KotlinComponent => "Kotlin Wasm Component（预览）",
            Self::KotlinService => "Kotlin 服务",
            Self::TypeScriptPages => "TypeScript 静态页面",
            Self::TypeScriptComponent => "TypeScript Wasm Component",
            Self::TypeScriptService => "Node.js 服务",
        }
    }
}

pub fn parse_runtime(value: &str) -> Result<PluginRuntimeOverride> {
    match value {
        "page-definition" => Ok(PluginRuntimeOverride::PageDefinition),
        "wasm-component" => Ok(PluginRuntimeOverride::WasmComponent),
        "process" => Ok(PluginRuntimeOverride::Process),
        "rust-source" => {
            bail!("Rust 源码插件无需 --runtime；实现插件 trait 后由 Dill 按 TypeId 自动聚合")
        }
        _ => bail!("插件模板覆盖必须是 page-definition、wasm-component 或 process: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_stable_default_template_to_each_language() -> Result<()> {
        assert_eq!(
            PluginTemplate::resolve(PluginLanguage::Rust, None)?,
            PluginTemplate::Fullstack(PluginLanguage::Rust)
        );
        assert_eq!(
            PluginTemplate::resolve(PluginLanguage::Kotlin, None)?,
            PluginTemplate::Fullstack(PluginLanguage::Kotlin)
        );
        assert_eq!(
            PluginTemplate::resolve(PluginLanguage::TypeScript, None)?,
            PluginTemplate::Fullstack(PluginLanguage::TypeScript)
        );
        Ok(())
    }

    #[test]
    fn resolves_advanced_template_overrides() -> Result<()> {
        assert_eq!(
            PluginTemplate::resolve(
                PluginLanguage::Kotlin,
                Some(PluginRuntimeOverride::WasmComponent)
            )?,
            PluginTemplate::KotlinComponent
        );
        assert_eq!(
            PluginTemplate::resolve(
                PluginLanguage::TypeScript,
                Some(PluginRuntimeOverride::PageDefinition)
            )?,
            PluginTemplate::TypeScriptPages
        );
        Ok(())
    }

    #[test]
    fn rejects_runtime_for_trait_driven_rust_plugins() {
        assert!(
            PluginTemplate::resolve(PluginLanguage::Rust, Some(PluginRuntimeOverride::Process))
                .is_err()
        );
    }

    #[test]
    fn runtime_override_excludes_rust_source() {
        let error = parse_runtime("rust-source").expect_err("rust-source 不应是模板覆盖选项");
        assert!(error.to_string().contains("Rust 源码插件无需 --runtime"));
        assert!(error.to_string().contains("TypeId"));

        let error = parse_runtime("native").expect_err("未知模板覆盖应被拒绝");
        assert!(!error.to_string().contains("rust-source"));
    }
}
