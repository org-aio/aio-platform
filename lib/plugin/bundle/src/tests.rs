use std::{collections::BTreeMap, fs};

use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::{Bundle, BundleManifest, validate_relative_path};

const MANIFEST: &str = r#"
schema_version = 2
[plugin.runtime]
artifact = "dist/plugin.wasm"
host_version = ">=2026.9.11"
[plugin.frontend]
path = "dist/frontend"
[plugin.capabilities]
database = true
[plugin.database]
migrations = "backend/migrations"
"#;

#[test]
fn validates_marketplace_parent_and_permissions() -> Result<()> {
    let metadata = "\n[plugin.marketplace]\ntitle='Screen'\nsummary='Workspace'\nlicense='MIT'\nparent='https://github.com/example/parent.git'\nparent_title='父插件'\n";
    let mut package = bundle();
    package.manifest.push_str(metadata);
    package.digest = package.content_digest();
    package.verify()?;
    assert_eq!(
        BundleManifest::parse(&package.manifest)?
            .plugin
            .marketplace
            .unwrap()
            .parent_title
            .as_deref(),
        Some("父插件")
    );
    assert!(
        BundleManifest::parse(
            &package
                .manifest
                .replace("parent_title='父插件'", "parent_title=' '")
        )
        .is_err()
    );
    assert!(
        BundleManifest::parse(
            &package
                .manifest
                .replace("parent='https://github.com/example/parent.git'", "")
        )
        .is_err()
    );
    package.git = "https://github.com/example/parent.git".into();
    package.digest = package.content_digest();
    assert!(package.verify().is_err());
    for parent in [
        "../parent",
        "https://github.com/user/parent.git?token=x",
        "https://user:secret@github.com/user/parent.git",
    ] {
        assert!(
            BundleManifest::parse(&format!(
                "{MANIFEST}{}",
                metadata.replace("https://github.com/example/parent.git", parent)
            ))
            .is_err()
        );
    }
    assert!(
        BundleManifest::parse(&MANIFEST.replace(
            "[plugin.runtime]",
            "[plugin]\npermissions=['*']\n[plugin.runtime]"
        ))
        .is_err()
    );
    Ok(())
}

fn bundle() -> Bundle {
    let files = BTreeMap::from([
        (
            "dist/plugin.wasm".into(),
            STANDARD.encode(b"\0asm\x0d\0\x01\0"),
        ),
        (
            "dist/frontend/index.html".into(),
            STANDARD.encode(b"<html>example</html>"),
        ),
        (
            "dist/frontend/@font/assets/font.woff2".into(),
            STANDARD.encode(b"font"),
        ),
        (
            "backend/migrations/0001.sql".into(),
            STANDARD.encode(b"CREATE TABLE counter (value BIGINT);"),
        ),
    ]);
    let mut bundle = Bundle {
        abi_version: 2,
        git: "https://example.com/team/example.git".into(),
        commit: "a".repeat(40),
        version: "1.0.0".into(),
        manifest: MANIFEST.into(),
        files,
        digest: String::new(),
    };
    bundle.digest = bundle.content_digest();
    bundle
}

#[test]
fn frontend_metadata_is_bound_to_verified_bytes() -> Result<()> {
    use sha2::{Digest, Sha256};
    let verified = bundle().verify()?;
    assert_eq!(verified.frontend_assets().len(), 2);
    for (name, bytes) in verified.frontend_files() {
        let asset = &verified.frontend_assets()[name];
        assert_eq!(asset.size, bytes.len());
        assert_eq!(asset.digest, format!("{:x}", Sha256::digest(bytes)));
    }
    assert!(!verified.frontend_assets().contains_key("dist/plugin.wasm"));
    Ok(())
}

#[test]
fn entire_release_roundtrips_deterministically() -> Result<()> {
    let original = bundle();
    let encoded = original.encode()?;
    assert_eq!(encoded, original.encode()?);
    let verified = Bundle::decode(&encoded)?.verify()?;
    assert_eq!(verified.digest(), original.digest);
    assert_eq!(
        verified.frontend("index.html"),
        Some(b"<html>example</html>".as_slice())
    );
    assert_eq!(
        verified.frontend("@font/assets/font.woff2"),
        Some(b"font".as_slice())
    );
    assert!(
        verified
            .frontend("../../backend/migrations/0001.sql")
            .is_none()
    );
    assert_eq!(
        verified.frontend_files().collect::<Vec<_>>(),
        vec![
            ("@font/assets/font.woff2", b"font".as_slice()),
            ("index.html", b"<html>example</html>".as_slice()),
        ]
    );
    assert_eq!(
        verified.migrations().collect::<Vec<_>>(),
        vec![("0001.sql", "CREATE TABLE counter (value BIGINT);")]
    );
    Ok(())
}

#[test]
fn all_content_and_provenance_are_bound_to_one_digest() {
    for field in [
        "frontend",
        "backend",
        "migration",
        "manifest",
        "version",
        "git",
        "commit",
    ] {
        let mut changed = bundle();
        match field {
            "frontend" => {
                changed.files.insert(
                    "dist/frontend/index.html".into(),
                    STANDARD.encode(b"new UI"),
                );
            }
            "backend" => {
                changed.files.insert(
                    "dist/plugin.wasm".into(),
                    STANDARD.encode(b"\0asm\x0d\0\x01\0new"),
                );
            }
            "migration" => {
                changed.files.insert(
                    "backend/migrations/0001.sql".into(),
                    STANDARD.encode(b"CREATE TABLE changed (id INT);"),
                );
            }
            "manifest" => changed.manifest.push('\n'),
            "version" => changed.version = "2.0.0".into(),
            "git" => changed.git = "https://example.com/other.git".into(),
            "commit" => changed.commit = "b".repeat(40),
            _ => unreachable!(),
        }
        assert_ne!(changed.content_digest(), bundle().digest, "{field}");
        assert!(changed.verify().is_err(), "{field}");
    }
}

#[test]
fn rejects_missing_undeclared_and_wrong_backend_files() {
    for path in ["dist/plugin.wasm", "backend/migrations/0001.sql"] {
        let mut candidate = bundle();
        candidate.files.remove(path);
        candidate.digest = candidate.content_digest();
        assert!(candidate.verify().is_err());
    }
    for bytes in [b"\0asm\x01\0\0\0".as_slice(), b"PKjar", b"#!/bin/sh"] {
        let mut candidate = bundle();
        candidate
            .files
            .insert("dist/plugin.wasm".into(), STANDARD.encode(bytes));
        candidate.digest = candidate.content_digest();
        assert!(candidate.verify().is_err());
    }
    let mut candidate = bundle();
    candidate
        .files
        .insert("install.sh".into(), STANDARD.encode(b"echo bad"));
    candidate.digest = candidate.content_digest();
    assert!(candidate.verify().is_err());
}

#[test]
fn rejects_legacy_manifests_install_scripts_and_overlapping_directories() {
    for manifest in [
        MANIFEST.replace("schema_version = 2", "schema_version = 1"),
        MANIFEST.replace("artifact =", "kind = \"rust-source\"\nartifact ="),
        format!("{MANIFEST}\n[plugin.build]\nscript = \"install.sh\""),
        MANIFEST.replace("backend/migrations", "dist/frontend/migrations"),
        MANIFEST.replace("database = true", "database = false"),
    ] {
        assert!(BundleManifest::parse(&manifest).is_err());
    }
}

#[test]
fn rejects_unpinned_sources_and_unsafe_paths() {
    for commit in ["main", "abcdef", &"A".repeat(40)] {
        let mut candidate = bundle();
        candidate.commit = commit.into();
        candidate.digest = candidate.content_digest();
        assert!(candidate.verify().is_err());
    }
    for git in [
        "http://example.com/a.git",
        "https://secret@example.com/a.git",
        "https://example.com/a.git?token=secret",
    ] {
        let mut candidate = bundle();
        candidate.git = git.into();
        candidate.digest = candidate.content_digest();
        assert!(candidate.verify().is_err());
    }
    for path in [
        "../index.html",
        "/index.html",
        "a//b",
        "a\\b",
        "a/%2e%2e/b",
        "file.js?secret",
        "file\0.html",
    ] {
        assert!(validate_relative_path(path).is_err(), "{path}");
    }
}

#[test]
fn rejects_archive_trailers_and_corruption() -> Result<()> {
    let encoded = bundle().encode()?;
    let mut trailer = encoded.clone();
    trailer.push(0);
    assert!(Bundle::decode(&trailer).is_err());
    assert!(Bundle::decode(&[encoded.clone(), encoded.clone()].concat()).is_err());
    assert!(Bundle::decode(&encoded[..encoded.len() - 1]).is_err());
    Ok(())
}

#[test]
fn rejects_file_count_and_file_directory_collisions() {
    let mut excessive = bundle();
    for index in 0..crate::model::MAX_FILES {
        excessive
            .files
            .insert(format!("dist/frontend/{index}.js"), String::new());
    }
    excessive.digest = excessive.content_digest();
    assert!(
        excessive
            .verify()
            .unwrap_err()
            .to_string()
            .contains("文件数量")
    );
    let mut collision = bundle();
    collision
        .files
        .insert("dist/frontend/index.html/child.js".into(), String::new());
    collision.digest = collision.content_digest();
    assert!(
        collision
            .verify()
            .unwrap_err()
            .to_string()
            .contains("路径冲突")
    );
    assert!(
        BundleManifest::parse(
            &MANIFEST.replace("path = \"dist/frontend\"", "path = \"dist/plugin.wasm\"")
        )
        .is_err()
    );
}

#[test]
fn author_directory_builds_without_executing_scripts() -> Result<()> {
    let root = tempfile::tempdir()?;
    let expected = bundle();
    fs::write(root.path().join("aio-plugin.toml"), MANIFEST)?;
    for (path, content) in &expected.files {
        let file = root.path().join(path);
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(file, STANDARD.decode(content)?)?;
    }
    fs::write(root.path().join("install.sh"), "exit 99")?;
    fs::write(
        root.path().join("backend/migrations/README.md"),
        "# Migrations",
    )?;
    let actual = Bundle::from_directory(
        root.path(),
        "aio-plugin.toml",
        expected.git.clone(),
        expected.commit.clone(),
        expected.version.clone(),
    )?;
    assert_eq!(actual.digest, expected.digest);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            root.path().join("aio-plugin.toml"),
            root.path().join("dist/frontend/link.html"),
        )?;
        assert!(
            Bundle::from_directory(
                root.path(),
                "aio-plugin.toml",
                expected.git,
                expected.commit,
                expected.version
            )
            .is_err()
        );
    }
    Ok(())
}
