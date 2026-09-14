use std::path::Path;

use anyhow::Result;
use az_plugin_manifest::PluginRuntime;

use super::scaffold::{TemplateFile, materialize};

const COMPONENT_FILES: &[TemplateFile] = &[
    TemplateFile {
        path: ".gitattributes",
        content: include_str!("../../templates/plugin/kotlin-component/.gitattributes"),
        executable: false,
    },
    TemplateFile {
        path: ".gitignore",
        content: include_str!("../../templates/plugin/kotlin-component/.gitignore"),
        executable: false,
    },
    TemplateFile {
        path: "aio-plugin.toml",
        content: include_str!("../../templates/plugin/kotlin-component/aio-plugin.toml"),
        executable: false,
    },
    TemplateFile {
        path: "project.yaml",
        content: include_str!("../../templates/plugin/kotlin-component/project.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-component/model/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/README.md",
        content: include_str!("../../templates/plugin/kotlin-component/model/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "model/src/__PACKAGE_PATH__/contract/PluginContract.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/model/src/site/addzero/aio/plugin/kmpcomponent/contract/PluginContract.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/src/__PACKAGE_PATH__/contract/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-component/model/src/site/addzero/aio/plugin/kmpcomponent/contract/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/test/__PACKAGE_PATH__/contract/PluginContractTest.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/model/test/site/addzero/aio/plugin/kmpcomponent/contract/PluginContractTest.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/test/__PACKAGE_PATH__/contract/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-component/model/test/site/addzero/aio/plugin/kmpcomponent/contract/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-component/component/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "component/README.md",
        content: include_str!("../../templates/plugin/kotlin-component/component/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/bindings/InternalPage.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/bindings/InternalPage.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/bindings/Page.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/bindings/Page.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/bindings/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/bindings/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/bindings/runtime/ComponentSupport.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/bindings/runtime/ComponentSupport.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/bindings/runtime/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/bindings/runtime/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/component/PageExports.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/component/PageExports.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "component/src/__PACKAGE_PATH__/component/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-component/component/src/site/addzero/aio/plugin/kmpcomponent/component/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "runtime/README.md",
        content: include_str!("../../templates/plugin/kotlin-component/runtime/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "runtime/sandbox-preview1-adapter.wat",
        content: include_str!(
            "../../templates/plugin/kotlin-component/runtime/sandbox-preview1-adapter.wat"
        ),
        executable: false,
    },
    TemplateFile {
        path: "scripts/README.md",
        content: include_str!("../../templates/plugin/kotlin-component/scripts/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "scripts/build-component.sh",
        content: include_str!("../../templates/plugin/kotlin-component/scripts/build-component.sh"),
        executable: true,
    },
    TemplateFile {
        path: "scripts/generate-bindings.sh",
        content: include_str!(
            "../../templates/plugin/kotlin-component/scripts/generate-bindings.sh"
        ),
        executable: true,
    },
    TemplateFile {
        path: "wit/page.wit",
        content: include_str!("../../templates/plugin/kotlin-component/wit/page.wit"),
        executable: false,
    },
];

const PAGE_FILES: &[TemplateFile] = &[
    TemplateFile {
        path: ".gitignore",
        content: include_str!("../../templates/plugin/kotlin-pages/.gitignore"),
        executable: false,
    },
    TemplateFile {
        path: "aio-plugin.toml",
        content: include_str!("../../templates/plugin/kotlin-pages/aio-plugin.toml"),
        executable: false,
    },
    TemplateFile {
        path: "project.yaml",
        content: include_str!("../../templates/plugin/kotlin-pages/project.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-pages/model/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/README.md",
        content: include_str!("../../templates/plugin/kotlin-pages/model/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "model/src/__PACKAGE_PATH__/PageDefinition.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/model/src/site/addzero/aio/plugin/kmppages/PageDefinition.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/src/__PACKAGE_PATH__/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/model/src/site/addzero/aio/plugin/kmppages/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/test/__PACKAGE_PATH__/PageDefinitionTest.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/model/test/site/addzero/aio/plugin/kmppages/PageDefinitionTest.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/test/__PACKAGE_PATH__/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/model/test/site/addzero/aio/plugin/kmppages/README.md"
        ),
        executable: false,
    },
    TemplateFile {
        path: "generator/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-pages/generator/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "generator/README.md",
        content: include_str!("../../templates/plugin/kotlin-pages/generator/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "generator/src/__PACKAGE_PATH__/Main.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/generator/src/site/addzero/aio/plugin/kmppages/Main.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "generator/src/__PACKAGE_PATH__/README.md",
        content: include_str!(
            "../../templates/plugin/kotlin-pages/generator/src/site/addzero/aio/plugin/kmppages/README.md"
        ),
        executable: false,
    },
];

const PROCESS_FILES: &[TemplateFile] = &[
    TemplateFile {
        path: ".gitignore",
        content: include_str!("../../templates/plugin/kotlin-process/.gitignore"),
        executable: false,
    },
    TemplateFile {
        path: "aio-plugin.toml",
        content: include_str!("../../templates/plugin/kotlin-process/aio-plugin.toml"),
        executable: false,
    },
    TemplateFile {
        path: "project.yaml",
        content: include_str!("../../templates/plugin/kotlin-process/project.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-process/model/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "model/README.md",
        content: include_str!("../../templates/plugin/kotlin-process/model/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "model/src/__PACKAGE_PATH__/RuntimeModel.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-process/model/src/site/addzero/aio/plugin/kmpservice/RuntimeModel.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "model/test/__PACKAGE_PATH__/RuntimeModelTest.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-process/model/test/site/addzero/aio/plugin/kmpservice/RuntimeModelTest.kt"
        ),
        executable: false,
    },
    TemplateFile {
        path: "service/module.yaml",
        content: include_str!("../../templates/plugin/kotlin-process/service/module.yaml"),
        executable: false,
    },
    TemplateFile {
        path: "service/README.md",
        content: include_str!("../../templates/plugin/kotlin-process/service/README.md"),
        executable: false,
    },
    TemplateFile {
        path: "service/src/__PACKAGE_PATH__/Main.kt",
        content: include_str!(
            "../../templates/plugin/kotlin-process/service/src/site/addzero/aio/plugin/kmpservice/Main.kt"
        ),
        executable: false,
    },
];

pub fn repository_plugin(
    path: &Path,
    name: &str,
    title: &str,
    runtime: PluginRuntime,
) -> Result<()> {
    match runtime {
        PluginRuntime::PageDefinition => page_plugin(path, name, title),
        PluginRuntime::WasmComponent => component_plugin(path, name, title),
        PluginRuntime::Process => process_plugin(path, name, title),
        _ => unreachable!("语言与运行目标已在初始化入口校验"),
    }
}

fn component_plugin(path: &Path, name: &str, title: &str) -> Result<()> {
    let package = format!("site.addzero.aio.plugin.{}", name.replace('-', "_"));
    let package_path = package.replace('.', "/");
    let replacements = [
        ("__PACKAGE_PATH__", package_path),
        ("__PACKAGE_NAME__", package),
        ("__PLUGIN_ID__", name.to_owned()),
        ("__TITLE_LITERAL__", serde_json::to_string(title)?),
    ];
    materialize(path, COMPONENT_FILES, &replacements)?;
    super::write(&path.join("README.md"), &component_readme(title))
}

fn page_plugin(path: &Path, name: &str, title: &str) -> Result<()> {
    let package = format!("site.addzero.aio.plugin.{}", name.replace('-', "_"));
    let package_path = package.replace('.', "/");
    let name_literal = serde_json::to_string(name)?;
    let about_literal = serde_json::to_string(&format!("{name}-about"))?;
    let title_literal = serde_json::to_string(title)?;
    let replacements = [
        ("__PACKAGE_PATH__", package_path),
        ("__PACKAGE_NAME__", package),
        ("__PRIMARY_ID__", name_literal),
        ("__ABOUT_ID__", about_literal),
        ("__TITLE__", title_literal),
    ];
    materialize(path, PAGE_FILES, &replacements)?;
    super::write(&path.join("README.md"), &page_readme(title))
}

fn process_plugin(path: &Path, name: &str, title: &str) -> Result<()> {
    let package = format!("site.addzero.aio.plugin.{}", name.replace('-', "_"));
    let package_path = package.replace('.', "/");
    let title_literal = serde_json::to_string(title)?;
    let replacements = [
        ("__PACKAGE_PATH__", package_path),
        ("site.addzero.aio.plugin.kmpservice", package),
        ("kmp-process", name.to_owned()),
        ("__MARKETPLACE_TITLE__", title_literal.clone()),
        ("\"KMP 服务\"", title_literal.clone()),
        ("\"Kotlin 进程插件 v2 已在线\"", title_literal),
    ];
    materialize(path, PROCESS_FILES, &replacements)?;
    super::write(&path.join("README.md"), &process_readme(title))
}

fn page_readme(title: &str) -> String {
    format!(
        "# {title}\n\n这是使用 Kotlin Toolchain 构建的 AIO `page-definition` 插件。页面模型位于 commonMain，并同时通过 JVM 与 wasmJs 编译；JVM 生成器只负责输出语言无关的安装产物。\n\n```bash\n./kotlin check\n./kotlin build -m model -p jvm -p wasmJs\n./kotlin run -m generator -p jvm -- dist/pages.json\naio plugin validate\n```\n\n构建完成后执行 `aio plugin package . --version 1.0.0`，再用来源绑定凭证执行 `aio plugin publish dist/plugin.aio-plugin` 即可在线更新。`dist/pages.json` 不要求提交到 Git，发布不依赖 CI。静态页面插件没有运行实例，也不能声明动作页面。\n"
    )
}

fn process_readme(title: &str) -> String {
    format!(
        "# {title}\n\n这是使用 Kotlin Toolchain 构建的 AIO `process` 插件。页面和请求模型位于 commonMain，JVM 服务只负责 HTTP 适配。\n\n```bash\n./kotlin check\n./kotlin build -m model -p wasmJs\n./kotlin package -m service -p jvm -f executable-jar\nmkdir -p dist\ncp build/tasks/_service_executableJarJvm/service-jvm-executable.jar dist/plugin.jar\naio plugin validate\n```\n\n构建完成后执行 `aio plugin package . --version 1.0.0`，再用来源绑定凭证执行 `aio plugin publish dist/plugin.aio-plugin` 即可在线更新。`dist/plugin.jar` 不要求提交到 Git，发布不依赖 CI；生产宿主不会执行 Kotlin 构建。\n"
    )
}

fn component_readme(title: &str) -> String {
    format!(
        "# {title}\n\n这是使用 Kotlin Toolchain 构建的 AIO `wasm-component` 插件。协议模型位于 commonMain，并同时通过 JVM、wasmJs 与 wasmWasi 编译；WIT 适配层只暴露 `definition` 与 `handle`。\n\n```bash\n./kotlin check\nWIT_BINDGEN=/path/to/wit-bindgen ./scripts/generate-bindings.sh --check\n./scripts/build-component.sh\naio plugin validate\n```\n\n构建完成后执行 `aio plugin package . --version 1.0.0`，再用来源绑定凭证执行 `aio plugin publish dist/plugin.aio-plugin` 即可在线更新。`dist/plugin.wasm` 不要求提交到 Git，发布不依赖 CI。当前 Kotlin Component 工具链仍是预览能力，仓库已锁定生成器版本且默认不授予网络、文件系统或数据库能力。\n"
    )
}
