use std::{fs, process::Command};

use tempfile::tempdir;

#[test]
fn initializes_every_template_without_tools_or_network() {
    let root = tempdir().unwrap();
    let templates: &[(&str, &[&str])] = &[
        ("rust", &[]),
        ("rust", &["--kind", "system"]),
        ("kotlin", &[]),
        ("kotlin", &["--runtime", "process"]),
        ("kotlin", &["--runtime", "page-definition"]),
        ("kotlin", &["--runtime", "wasm-component"]),
        ("typescript", &[]),
        ("typescript", &["--framework", "nuxt"]),
        ("typescript", &["--framework", "next"]),
        ("rust", &["--framework", "topcoat"]),
        ("typescript", &["--runtime", "process"]),
        ("typescript", &["--runtime", "page-definition"]),
        ("typescript", &["--runtime", "wasm-component"]),
    ];
    for profile in ["china", "global"] {
        for (index, (language, options)) in templates.iter().enumerate() {
            let path = root.path().join(format!("plugin-{profile}-{index}"));
            let result = Command::new(env!("CARGO_BIN_EXE_aio"))
                .args(["plugin", "init"])
                .arg(&path)
                .args(["--language", language, "--network", profile])
                .args(*options)
                .env("PATH", "")
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .env("JAVA_HOME", "/does/not/exist")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(path.join("NETWORK.md").is_file());
            let config = fs::read_to_string(path.join(if *language == "rust" {
                ".cargo/config.toml"
            } else {
                ".npmrc"
            }))
            .unwrap();
            assert_eq!(
                config.contains(if *language == "rust" {
                    "rsproxy.cn"
                } else {
                    "npmmirror.com"
                }),
                profile == "china"
            );
            if *language == "kotlin" {
                assert!(path.join("kotlin").is_file());
                assert!(path.join("kotlin.bat").is_file());
                assert!(path.join(".aio/toolchain/artifacts.tsv").is_file());
                let bytes = fs::read(path.join(".aio/toolchain/bootstrap.ps1")).unwrap();
                assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
                let repositories =
                    fs::read_to_string(path.join("network.module-template.yaml")).unwrap();
                assert_eq!(repositories.contains("huaweicloud.com"), profile == "china");
            }
        }
    }
}

#[test]
fn rejects_bad_network_before_creating_directory() {
    let root = tempdir().unwrap();
    for command in [&["init"][..], &["plugin", "init"][..]] {
        let destination = root.path().join("invalid-profile");
        let output = Command::new(env!("CARGO_BIN_EXE_aio"))
            .args(command)
            .arg(&destination)
            .args(["--network", "invalid"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!destination.exists());
        assert!(String::from_utf8_lossy(&output.stderr).contains("china 或 global"));
    }
}
