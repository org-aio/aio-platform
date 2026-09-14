use std::{fs, path::Path};

use anyhow::Result;
use include_dir::{Dir, include_dir};

use super::NetworkProfile;
use crate::initialize::PluginLanguage;

static TOOLCHAIN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/toolchain");

pub(in crate::initialize) fn configure(
    root: &Path,
    language: PluginLanguage,
    profile: NetworkProfile,
    web: bool,
) -> Result<()> {
    let write = |path: &str, content: &str| -> Result<()> {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, content)?;
        Ok(())
    };
    write(
        "NETWORK.md",
        include_str!("../../../templates/toolchain/README.md"),
    )?;
    match language {
        PluginLanguage::Rust => {
            let source = if profile == NetworkProfile::China {
                "[source.crates-io]\nreplace-with = \"aio-mirror\"\n\n[source.aio-mirror]\nregistry = \"sparse+https://rsproxy.cn/index/\"\n\n"
            } else {
                ""
            };
            write(
                ".cargo/config.toml",
                &format!(
                    "{source}[net]\nretry = 3\ngit-fetch-with-cli = true\n\n[http]\ntimeout = 60\n"
                ),
            )?;
        }
        PluginLanguage::TypeScript | PluginLanguage::Kotlin => {
            let registry = match profile {
                NetworkProfile::China => "https://registry.npmmirror.com",
                NetworkProfile::Global => "https://registry.npmjs.org",
            };
            write(
                ".npmrc",
                &format!("registry={registry}\nfetch-retries=3\nfetch-timeout=60000\n"),
            )?;
            if language == PluginLanguage::Kotlin {
                for file in TOOLCHAIN.files() {
                    if matches!(file.path().to_str(), Some("wrapper.sh" | "wrapper.bat")) {
                        continue;
                    }
                    let content = std::str::from_utf8(file.contents())?;
                    let content = if file.path().extension().is_some_and(|ext| ext == "ps1") {
                        format!("\u{feff}{content}")
                    } else {
                        content.to_owned()
                    };
                    write(
                        &format!(".aio/toolchain/{}", file.path().display()),
                        &content,
                    )?;
                }
                write(
                    "kotlin",
                    include_str!("../../../templates/toolchain/wrapper.sh"),
                )?;
                write(
                    "kotlin.bat",
                    &include_str!("../../../templates/toolchain/wrapper.bat").replace('\n', "\r\n"),
                )?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    fs::set_permissions(root.join("kotlin"), fs::Permissions::from_mode(0o755))?;
                }
                write(".aio/toolchain/profile", profile.name())?;
                write(".aio/toolchain/web", if web { "true" } else { "false" })?;
                let mirrors = if profile == NetworkProfile::China {
                    "  - id: aioHuawei\n    url: https://repo.huaweicloud.com/repository/maven\n  - id: aioAliyun\n    url: https://maven.aliyun.com/repository/public\n  - id: aioGoogle\n    url: https://maven.aliyun.com/repository/google\n"
                } else {
                    ""
                };
                write(
                    "network.module-template.yaml",
                    &format!(
                        "repositories:\n{mirrors}  - id: mavenCentral\n    url: https://repo.maven.apache.org/maven2\n  - id: mavenGoogle\n    url: https://dl.google.com/dl/android/maven2\n"
                    ),
                )?;
            }
        }
    }
    Ok(())
}
