use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::{
    ConventionFileResult, PageDefinition, PageRendererDefinition, ProgramDefinition,
    convention_page_path,
};

#[derive(Clone, Debug)]
pub struct ConventionFileGenerator {
    app_dir: PathBuf,
}

impl ConventionFileGenerator {
    #[must_use]
    pub fn workspace_app() -> Self {
        let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self { app_dir }
    }

    pub fn generate(
        &self,
        program: &ProgramDefinition,
        page: &PageDefinition,
    ) -> Result<ConventionFileResult> {
        ensure!(
            matches!(page.renderer, PageRendererDefinition::ConventionFile),
            "只有约定文件页面可以生成 Rust 文件"
        );
        let relative_path = convention_page_path(&program.name, &page.name);
        let path = self.app_dir.join(&relative_path);
        ensure_inside_app(&self.app_dir, &path)?;
        if path.exists() {
            return Ok(ConventionFileResult {
                path: format!("app/{relative_path}"),
                created: false,
            });
        }
        let parent = path.parent().context("约定页面文件缺少父目录")?;
        std::fs::create_dir_all(parent).context("创建约定页面目录失败")?;
        let source = convention_page_source();
        std::fs::write(&path, source)
            .with_context(|| format!("写入约定页面文件失败: {}", path.display()))?;
        Ok(ConventionFileResult {
            path: format!("app/{relative_path}"),
            created: true,
        })
    }
}

fn ensure_inside_app(app_dir: &Path, path: &Path) -> Result<()> {
    let app_dir = app_dir.canonicalize().context("解析 app 目录失败")?;
    let parent = path.parent().context("约定页面路径缺少父目录")?;
    std::fs::create_dir_all(parent).context("创建约定页面父目录失败")?;
    let parent = parent.canonicalize().context("解析约定页面父目录失败")?;
    ensure!(parent.starts_with(app_dir), "约定页面路径越出 app 目录");
    Ok(())
}

fn convention_page_source() -> String {
    r#"use std::sync::Arc;

use dioxus::prelude::*;
use rudi::Singleton;
use studio::{
    ConventionPageContext, ConventionPageProvider, DynConventionPageProvider,
};

#[derive(Clone, Debug, Default)]
struct Page;

impl ConventionPageProvider for Page {
    fn key(&self) -> &'static str {
        module_path!()
    }

    fn render(&self, context: ConventionPageContext) -> Element {
        rsx! {
            section { class: "p-6",
                h2 { class: "text-lg font-semibold", "{context.page.title}" }
            }
        }
    }
}

#[Singleton(name = module_path!())]
fn convention_page() -> DynConventionPageProvider {
    Arc::new(Page)
}
"#
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_source_registers_the_convention_key() {
        let source = convention_page_source();
        assert!(source.contains("module_path!()"));
        assert!(source.contains("ConventionPageProvider"));
    }
}
