#![forbid(unsafe_code)]

mod initialize;
mod marketplace;
mod repository_plugin;

use std::{env, path::PathBuf};

use anyhow::{Result, bail};
use initialize::{
    ApplicationOptions, NetworkProfile, PluginLanguage, PluginTemplate, RepositoryPluginOptions,
    parse_runtime,
};
use repository_plugin::{PackageOptions, PluginSource, PublicationOptions};

fn main() -> Result<()> {
    run(env::args().skip(1).collect())
}

fn run(arguments: Vec<String>) -> Result<()> {
    let Some(command) = arguments.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    match command {
        "init" => initialize_application(&arguments[1..]),
        "marketplace" => run_marketplace_command(&arguments[1..]),
        "plugin" => run_plugin_command(&arguments[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => bail!("未知命令: {command}\n\n{}", usage()),
    }
}

fn run_marketplace_command(arguments: &[String]) -> Result<()> {
    let Some(command) = arguments.first().map(String::as_str) else {
        bail!("缺少市场命令\n\n{}", usage());
    };
    match (command, &arguments[1..]) {
        ("build", []) => marketplace::build(
            &PathBuf::from("marketplace/registry"),
            &PathBuf::from("marketplace/index.json"),
        ),
        ("build", [registry, output]) => {
            marketplace::build(&PathBuf::from(registry), &PathBuf::from(output))
        }
        ("build", _) => bail!("marketplace build 只接受可选的 <registry> <output>"),
        _ => bail!("未知市场命令: {command}\n\n{}", usage()),
    }
}

fn initialize_application(arguments: &[String]) -> Result<()> {
    initialize::application(parse_init_arguments(arguments)?)
}

fn run_plugin_command(arguments: &[String]) -> Result<()> {
    let Some(command) = arguments.first().map(String::as_str) else {
        bail!("缺少插件命令\n\n{}", usage());
    };

    match command {
        "init" if arguments[1..].iter().any(|argument| is_help(argument)) => {
            println!("{}", plugin_init_usage());
            Ok(())
        }
        "init" => initialize::repository_plugin(parse_plugin_init_arguments(&arguments[1..])?),
        "install" => {
            let source = parse_plugin_source(&arguments[1..])?;
            repository_plugin::install(&env::current_dir()?, source)
        }
        "publish" if arguments[1..].iter().any(|argument| is_help(argument)) => {
            println!("{}", plugin_publish_usage());
            Ok(())
        }
        "publish" => repository_plugin::publish(parse_plugin_publish_arguments(&arguments[1..])?),
        "package" if arguments[1..].iter().any(|argument| is_help(argument)) => {
            println!("{}", plugin_package_usage());
            Ok(())
        }
        "package" => repository_plugin::package(parse_plugin_package_arguments(&arguments[1..])?),
        "uninstall" => {
            let git = exactly_one(&arguments[1..], "缺少要卸载的 Git 地址")?;
            repository_plugin::uninstall(&env::current_dir()?, git)
        }
        "sync" => no_arguments(&arguments[1..], || {
            repository_plugin::sync(&env::current_dir()?)
        }),
        "list" => no_arguments(&arguments[1..], || {
            repository_plugin::list(&env::current_dir()?)
        }),
        "validate" => match &arguments[1..] {
            [] => repository_plugin::validate(&env::current_dir()?),
            [path] => repository_plugin::validate(&PathBuf::from(path)),
            _ => bail!("plugin validate 只接受可选的 <仓库目录>"),
        },
        "schema" => match &arguments[1..] {
            [] => repository_plugin::write_schemas(&PathBuf::from("schemas")),
            [path] => repository_plugin::write_schemas(&PathBuf::from(path)),
            _ => bail!("plugin schema 只接受可选的 <输出目录>"),
        },
        _ => bail!("未知插件命令: {command}\n\n{}", usage()),
    }
}

fn parse_init_arguments(arguments: &[String]) -> Result<ApplicationOptions> {
    let Some(path) = arguments.first() else {
        bail!("缺少初始化目录");
    };
    let mut name = None;
    let mut title = None;
    let mut network = NetworkProfile::default();
    let mut index = 1;
    while index < arguments.len() {
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("选项 {} 缺少值", arguments[index]))?;
        match arguments[index].as_str() {
            "--name" => name = Some(value.clone()),
            "--title" => title = Some(value.clone()),
            "--network" => network = NetworkProfile::parse(value)?,
            option => bail!("未知初始化选项: {option}"),
        }
        index += 2;
    }
    Ok(ApplicationOptions {
        path: PathBuf::from(path),
        name,
        title,
        network,
    })
}

fn parse_plugin_init_arguments(arguments: &[String]) -> Result<RepositoryPluginOptions> {
    let Some(path) = arguments.first() else {
        bail!("缺少初始化目录");
    };
    let mut name = None;
    let mut title = None;
    let mut language = None;
    let mut kind = None;
    let mut runtime = None;
    let mut network = NetworkProfile::default();
    let mut index = 1;
    while index < arguments.len() {
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("选项 {} 缺少值", arguments[index]))?;
        match arguments[index].as_str() {
            "--name" => name = Some(value.clone()),
            "--title" => title = Some(value.clone()),
            "--language" => language = Some(PluginLanguage::parse(value)?),
            "--kind" => kind = Some(value.as_str()),
            "--runtime" => runtime = Some(parse_runtime(value)?),
            "--network" => network = NetworkProfile::parse(value)?,
            option => bail!("未知插件初始化选项: {option}"),
        }
        index += 2;
    }
    let language = language.unwrap_or_default();
    let template = match kind {
        None | Some("fullstack") if runtime.is_none() => PluginTemplate::Fullstack(language),
        Some("system") if language == PluginLanguage::Rust && runtime.is_none() => {
            PluginTemplate::Rust
        }
        None | Some("runtime") => PluginTemplate::resolve(language, runtime)?,
        _ => bail!(
            "--kind 必须为 fullstack、system（仅 Rust）或 runtime；fullstack/system 不能指定 --runtime"
        ),
    };
    Ok(RepositoryPluginOptions {
        path: PathBuf::from(path),
        name,
        title,
        template,
        network,
    })
}

fn parse_plugin_source(arguments: &[String]) -> Result<PluginSource> {
    let Some(git) = arguments.first() else {
        bail!("缺少要安装的 Git 地址");
    };
    let mut rev = None;
    match &arguments[1..] {
        [] => {}
        [option, value] if option == "--rev" => rev = Some(value.clone()),
        _ => bail!("插件安装只接受可选的 --rev <版本>"),
    }
    Ok(PluginSource {
        git: git.clone(),
        rev,
    })
}

fn parse_plugin_publish_arguments(arguments: &[String]) -> Result<PublicationOptions> {
    let (options, _) = parse_plugin_release_arguments(arguments, false)?;
    Ok(options)
}

fn parse_plugin_package_arguments(arguments: &[String]) -> Result<PackageOptions> {
    let (options, output) = parse_plugin_release_arguments(arguments, true)?;
    Ok(PackageOptions {
        root: options.root,
        git: options.git,
        version: options
            .version
            .ok_or_else(|| anyhow::anyhow!("plugin package 需要 --version <SemVer>"))?,
        output,
    })
}

fn parse_plugin_release_arguments(
    arguments: &[String],
    packaging: bool,
) -> Result<(PublicationOptions, Option<PathBuf>)> {
    let mut root = None;
    let mut git = None;
    let mut version = None;
    let mut output = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--git" | "--version" | "-o" | "--output" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("选项 {} 缺少值", arguments[index]))?;
                match arguments[index].as_str() {
                    "--git" if git.is_none() => git = Some(value.clone()),
                    "--version" if version.is_none() => version = Some(value.clone()),
                    "-o" | "--output" if packaging && output.is_none() => {
                        output = Some(PathBuf::from(value))
                    }
                    "-o" | "--output" if !packaging => {
                        bail!("plugin publish 不接受输出文件选项，请使用 plugin package")
                    }
                    "--git" | "--version" | "-o" | "--output" => {
                        bail!("选项 {} 重复", arguments[index])
                    }
                    _ => unreachable!(),
                }
                index += 2;
            }
            value if !value.starts_with('-') && root.is_none() => {
                root = Some(PathBuf::from(value));
                index += 1;
            }
            option => bail!("未知插件打包或发布选项: {option}"),
        }
    }
    if packaging && root.is_none() {
        bail!("plugin package 缺少插件目录");
    }
    Ok((
        PublicationOptions {
            root: root.map(Ok).unwrap_or_else(env::current_dir)?,
            git,
            version,
        },
        output,
    ))
}

fn exactly_one<'a>(arguments: &'a [String], missing: &str) -> Result<&'a str> {
    match arguments {
        [] => bail!("{missing}"),
        [value] => Ok(value),
        _ => bail!("命令只接受一个参数"),
    }
}

fn no_arguments(action_arguments: &[String], action: impl FnOnce() -> Result<()>) -> Result<()> {
    if !action_arguments.is_empty() {
        bail!("命令不接受参数");
    }
    action()
}

fn print_usage() {
    println!("{}", usage());
}

fn is_help(argument: &str) -> bool {
    matches!(argument, "--help" | "-h")
}

fn usage() -> &'static str {
    "用法:\n  aio init <目录> [--name <包名>] [--title <标题>] [--network <china|global>]\n  aio plugin init <目录> [--name <包名>] [--title <插件标题>] [--language <rust|kotlin|typescript>] [--network <china|global>]\n  aio plugin init --help\n  aio plugin install <git> [--rev <分支、标签或提交>]\n  aio plugin package <目录> --version <SemVer> [--git <HTTPS Git>] [-o <文件.aio-plugin>]\n  aio plugin publish [<目录或文件.aio-plugin>] [--git <HTTPS Git>] [--version <SemVer>]\n  aio plugin uninstall <git>\n  aio plugin sync\n  aio plugin list\n  aio plugin validate [<仓库目录>]\n  aio plugin schema [<输出目录>]\n  aio marketplace build [<registry> <output>]"
}

fn plugin_init_usage() -> &'static str {
    "用法:\n  aio plugin init <目录> [--name <包名>] [--title <标题>] [--language <rust|kotlin|typescript>] [--network <china|global>] [--kind <fullstack|system|runtime>]\n\n默认国内依赖源，初始化不联网；--network global 使用官方源。\n默认生成全栈插件，包含前端、后端、共享模型与自动发布配置。推送默认分支后自动构建上架。\n  --kind system   Rust 系统源码插件，使用 trait + Dill/TypeId\n\n高级模板覆盖（Kotlin/TypeScript）:\n  --runtime page-definition   静态页面\n  --runtime wasm-component    Component\n  --runtime process           JVM/Node 服务"
}
fn plugin_publish_usage() -> &'static str {
    "用法:\n  aio plugin publish [<目录或文件.aio-plugin>] [--git <HTTPS Git>] [--version <SemVer>]\n\n默认直接发布到官方插件中心，不依赖 CI。目录可从自己的 origin 推导来源，并从 Cargo.toml 或 package.json 推导版本。已有插件包保留包内来源和版本。\n\n环境变量:\n  AIO_PLUGIN_PUBLISH_TOKEN   插件市场创建的来源绑定发布凭证\n  AIO_PLUGIN_PUBLISH_URL     可选，覆盖官方插件中心发布接口\n\n发布前校验包的清单、版本和内容摘要。构建产物不需要提交到 Git。"
}

fn plugin_package_usage() -> &'static str {
    "用法:\n  aio plugin package <目录> --version <SemVer> [--git <HTTPS Git>] [-o <文件.aio-plugin>]\n\n从已经构建的插件目录生成可搬运的二进制包。来源默认读取该目录自己的 origin；没有 Git 仓库时指定 --git 即可。默认输出 <目录>/dist/plugin.aio-plugin。\n\n示例:\n  aio plugin package hello --git https://example.com/team/hello.git --version 1.0.0\n  aio plugin publish hello/dist/plugin.aio-plugin"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multilingual_plugin_template_options() -> Result<()> {
        let options = parse_plugin_init_arguments(&[
            "plugin".to_owned(),
            "--language".to_owned(),
            "kotlin".to_owned(),
            "--runtime".to_owned(),
            "process".to_owned(),
        ])?;

        assert_eq!(options.path, PathBuf::from("plugin"));
        assert_eq!(options.template, PluginTemplate::KotlinService);
        Ok(())
    }

    #[test]
    fn derives_runtime_from_language() -> Result<()> {
        let options = parse_plugin_init_arguments(&[
            "plugin".to_owned(),
            "--language".to_owned(),
            "typescript".to_owned(),
        ])?;

        assert_eq!(
            options.template,
            PluginTemplate::Fullstack(PluginLanguage::TypeScript)
        );
        Ok(())
    }

    #[test]
    fn rejects_language_runtime_mismatch() {
        let result = parse_plugin_init_arguments(&[
            "plugin".to_owned(),
            "--language".to_owned(),
            "rust".to_owned(),
            "--runtime".to_owned(),
            "process".to_owned(),
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_redundant_runtime_for_rust_plugin() {
        let result = parse_plugin_init_arguments(&[
            "plugin".to_owned(),
            "--runtime".to_owned(),
            "process".to_owned(),
        ]);

        let error = result.expect_err("Rust 插件不应要求运行目标");
        assert!(error.to_string().contains("trait"));
        assert!(error.to_string().contains("TypeId"));
    }

    #[test]
    fn primary_usage_hides_runtime_override() {
        let primary_command = usage()
            .lines()
            .find(|line| line.starts_with("  aio plugin init <"))
            .expect("主帮助应展示插件初始化命令");
        assert!(!primary_command.contains("--runtime"));
        assert!(plugin_init_usage().contains("高级模板覆盖"));
        assert!(plugin_init_usage().contains("--kind system"));
    }

    #[test]
    fn detailed_help_matches_runtime_override_parser() {
        for runtime in ["page-definition", "wasm-component", "process"] {
            assert!(parse_runtime(runtime).is_ok());
            let option = format!("--runtime {runtime}");
            assert!(plugin_init_usage().contains(option.as_str()));
        }
        assert!(parse_runtime("rust-source").is_err());
        assert!(!plugin_init_usage().contains("rust-source"));
    }

    #[test]
    fn plain_help_can_be_used_as_plugin_directory() {
        assert!(!is_help("help"));
        assert!(is_help("--help"));
        assert!(is_help("-h"));
    }

    #[test]
    fn parses_plugin_publish_defaults_and_overrides() -> Result<()> {
        let options = parse_plugin_publish_arguments(&[
            "plugin".to_owned(),
            "--git".to_owned(),
            "https://example.com/plugin.git".to_owned(),
            "--version".to_owned(),
            "1.2.3".to_owned(),
        ])?;
        assert_eq!(options.root, PathBuf::from("plugin"));
        assert_eq!(
            options.git.as_deref(),
            Some("https://example.com/plugin.git")
        );
        assert_eq!(options.version.as_deref(), Some("1.2.3"));
        assert!(!plugin_publish_usage().contains("GITHUB_SHA"));
        assert!(parse_plugin_publish_arguments(&["--rev".to_owned(), "a".repeat(40)]).is_err());
        Ok(())
    }

    #[test]
    fn parses_binary_package_output_and_requires_version() -> Result<()> {
        let options = parse_plugin_package_arguments(&[
            "hello".to_owned(),
            "--version".to_owned(),
            "1.0.0".to_owned(),
            "-o".to_owned(),
            "hello.aio-plugin".to_owned(),
        ])?;
        assert_eq!(options.root, PathBuf::from("hello"));
        assert_eq!(options.output, Some(PathBuf::from("hello.aio-plugin")));
        assert!(parse_plugin_package_arguments(&["hello".to_owned()]).is_err());
        assert!(parse_plugin_publish_arguments(&["-o".to_owned(), "out".to_owned()]).is_err());
        assert!(
            parse_plugin_publish_arguments(&[
                "--git".to_owned(),
                "one".to_owned(),
                "--git".to_owned(),
                "two".to_owned()
            ])
            .is_err()
        );
        Ok(())
    }
}
