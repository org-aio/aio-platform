use std::{fs, io::Write};

use tempfile::{NamedTempFile, tempdir};

use super::*;

const RUNTIME: &str = r#"
[plugin.runtime]
kind = "page-definition"
artifact = "dist/pages.json"
"#;

#[test]
fn accepts_parent_with_ordered_subplugins() -> Result<()> {
    let manifest = parse_manifest(&format!(
        "{RUNTIME}\n[[plugin.subplugins]]\nid='profile'\npages=['profile']\n[[plugin.subplugins]]\nid='account'\ndependencies=['profile']\naccount_actions=['profile']"
    ))?;
    assert_eq!(manifest.plugin.subplugins.len(), 2);
    Ok(())
}

#[test]
fn validates_marketplace_metadata() -> Result<()> {
    let manifest = parse_manifest(&format!(
        "{RUNTIME}\n[plugin.marketplace]\ntitle='Counter'\nsummary='Tenant counter'\nlicense='MIT'\ntags=['example', 'wasm']"
    ))?;
    assert_eq!(
        manifest
            .plugin
            .marketplace
            .as_ref()
            .expect("市场元数据必须保留")
            .title,
        "Counter"
    );

    let error = parse_manifest(&format!(
        "{RUNTIME}\n[plugin.marketplace]\ntitle='Counter'\nsummary='Tenant counter'\nlicense='MIT'\ntags=[]"
    ))
    .expect_err("空市场标签必须失败");
    assert!(error.to_string().contains("市场标签"));
    Ok(())
}

#[test]
fn rejects_missing_dependency_cycle_and_duplicate_contribution() {
    let missing = parse_manifest(&format!(
        "{RUNTIME}\n[[plugin.subplugins]]\nid='account'\ndependencies=['missing']"
    ))
    .expect_err("缺失依赖必须失败");
    assert!(missing.to_string().contains("依赖不存在"));

    let cycle = parse_manifest(&format!(
        "{RUNTIME}\n[[plugin.subplugins]]\nid='a'\ndependencies=['b']\n[[plugin.subplugins]]\nid='b'\ndependencies=['a']"
    ))
    .expect_err("循环依赖必须失败");
    assert!(cycle.to_string().contains("循环"));

    let duplicate = parse_manifest(&format!(
        "{RUNTIME}\n[[plugin.subplugins]]\nid='a'\nroutes=['echo']\n[[plugin.subplugins]]\nid='b'\nroutes=['echo']"
    ))
    .expect_err("重复路由必须失败");
    assert!(duplicate.to_string().contains("路由声明重复"));
}

#[test]
fn rejects_blank_page_fields() {
    let pages = [PageDefinition {
        id: "page".to_owned(),
        label: " ".to_owned(),
        icon: None,
        scene: crate::SceneDefinition {
            id: "scene".to_owned(),
            label: "Scene".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: None,
        body: PageBody::Text {
            title: "Title".to_owned(),
            content: "Content".to_owned(),
        },
    }];
    let error = validate_page_definitions(&pages).expect_err("空标题必须失败");
    assert!(error.to_string().contains("页面标题"));
}

#[test]
fn accepts_empty_pages_for_service_only_runtime() -> Result<()> {
    validate_page_definitions(&[])
}

#[test]
fn validates_nested_menu_paths_and_rejects_conflicts() -> Result<()> {
    let page = |id: &str, group_label: &str| PageDefinition {
        id: id.to_owned(),
        label: id.to_owned(),
        icon: None,
        scene: crate::SceneDefinition {
            id: "system".to_owned(),
            label: "系统".to_owned(),
        },
        menu_path: vec![crate::MenuGroupDefinition {
            id: "system-management".to_owned(),
            label: group_label.to_owned(),
            icon: Some("settings".to_owned()),
        }],
        required_permission: None,
        body: PageBody::Text {
            title: id.to_owned(),
            content: id.to_owned(),
        },
    };
    validate_page_definitions(&[page("users", "系统管理"), page("roles", "系统管理")])?;
    let error = validate_page_definitions(&[page("users", "系统管理"), page("roles", "基础设施")])
        .expect_err("同一分组不能声明冲突标题");
    assert!(error.to_string().contains("展示信息不一致"));

    let mut cyclic = page("dictionary", "系统管理");
    cyclic.menu_path.push(cyclic.menu_path[0].clone());
    assert!(validate_page_definitions(&[cyclic]).is_err());
    Ok(())
}

#[test]
fn validates_runtime_page_actions() -> Result<()> {
    let mut pages = [PageDefinition {
        id: "actions".to_owned(),
        label: "Actions".to_owned(),
        icon: None,
        scene: crate::SceneDefinition {
            id: "examples".to_owned(),
            label: "Examples".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: None,
        body: PageBody::Actions {
            title: "Counter".to_owned(),
            content: "0".to_owned(),
            state: [("count".to_owned(), serde_json::json!(0))]
                .into_iter()
                .collect(),
            actions: vec![crate::PageActionDefinition {
                id: "increment".to_owned(),
                label: "+1".to_owned(),
            }],
        },
    }];
    validate_page_definitions(&pages)?;

    let PageBody::Actions { actions, .. } = &mut pages[0].body else {
        unreachable!();
    };
    actions.push(actions[0].clone());
    let error = validate_page_definitions(&pages).expect_err("重复页面动作必须失败");
    assert!(error.to_string().contains("页面动作 id 重复"));
    Ok(())
}

#[test]
fn rejects_invalid_runtime_page_state() {
    let page = |state| PageDefinition {
        id: "actions".to_owned(),
        label: "Actions".to_owned(),
        icon: None,
        scene: crate::SceneDefinition {
            id: "examples".to_owned(),
            label: "Examples".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: None,
        body: PageBody::Actions {
            title: "Counter".to_owned(),
            content: "0".to_owned(),
            state,
            actions: vec![crate::PageActionDefinition {
                id: "increment".to_owned(),
                label: "+1".to_owned(),
            }],
        },
    };

    let blank = [(" ".to_owned(), serde_json::json!(0))]
        .into_iter()
        .collect();
    assert!(validate_page_definitions(&[page(blank)]).is_err());

    let oversized = [("content".to_owned(), serde_json::json!("x".repeat(65_536)))]
        .into_iter()
        .collect();
    let error = validate_page_definitions(&[page(oversized)]).expect_err("超大页面状态必须失败");
    assert!(error.to_string().contains("动作页状态不能超过"));
}

#[test]
fn rejects_runtime_actions_in_static_page_artifact() -> Result<()> {
    let repository = tempdir()?;
    fs::create_dir(repository.path().join("dist"))?;
    fs::write(
        repository.path().join("aio-plugin.toml"),
        format!("{RUNTIME}\n[[plugin.subplugins]]\nid='actions'\npages=['actions']"),
    )?;
    fs::write(
        repository.path().join("dist/pages.json"),
        r#"[{"id":"actions","label":"Actions","icon":null,"scene":{"id":"examples","label":"Examples"},"required_permission":null,"body":{"kind":"actions","title":"Counter","content":"0","actions":[{"id":"increment","label":"+1"}]}}]"#,
    )?;

    let error =
        validate_repository(repository.path()).expect_err("静态页面不能声明需要运行时处理的动作");
    assert!(error.to_string().contains("不能声明需要运行时处理"));
    Ok(())
}

#[test]
fn rejects_account_action_without_a_declared_page() -> Result<()> {
    let manifest = parse_manifest(&format!(
        "{RUNTIME}\n[[plugin.subplugins]]\nid='account'\npages=['profile']\naccount_actions=['missing']"
    ))?;
    let pages = [PageDefinition {
        id: "profile".to_owned(),
        label: "Profile".to_owned(),
        icon: None,
        scene: crate::SceneDefinition {
            id: "account".to_owned(),
            label: "Account".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: None,
        body: PageBody::Text {
            title: "Profile".to_owned(),
            content: "Profile".to_owned(),
        },
    }];

    let error =
        validate_declared_pages(&manifest, &pages).expect_err("账户动作不能指向不存在的页面");
    assert!(error.to_string().contains("账户动作必须指向"));
    Ok(())
}

#[test]
fn rejects_non_component_artifact() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"not wasm")?;
    let bytes = fs::read(file.path())?;
    assert!(validate_wasm_component(&bytes).is_err());
    Ok(())
}

#[test]
fn validates_page_artifact_against_subplugin_declarations() -> Result<()> {
    let repository = tempdir()?;
    fs::create_dir(repository.path().join("dist"))?;
    fs::write(
        repository.path().join("aio-plugin.toml"),
        format!("{RUNTIME}\n[[plugin.subplugins]]\nid='counter'\npages=['counter']"),
    )?;
    fs::write(
        repository.path().join("dist/pages.json"),
        r#"[{"id":"counter","label":"Counter","icon":null,"scene":{"id":"examples","label":"Examples"},"required_permission":null,"body":{"kind":"counter","title":"Counter","button":"+1"}}]"#,
    )?;

    let report = validate_repository(repository.path())?;
    assert_eq!(report.runtime, PluginRuntime::PageDefinition);
    assert_eq!(report.page_count, 1);

    fs::write(
        repository.path().join("aio-plugin.toml"),
        format!("{RUNTIME}\n[[plugin.subplugins]]\nid='other'\npages=['other']"),
    )?;
    let error =
        validate_repository(repository.path()).expect_err("清单与 PageDefinition 不一致必须失败");
    assert!(error.to_string().contains("不一致"));
    Ok(())
}

#[test]
fn requires_explicit_process_lifecycle_contract() -> Result<()> {
    let missing = parse_manifest("[plugin.runtime]\nkind='process'\nartifact='dist/plugin.jar'\n")
        .expect_err("缺少进程启动契约必须失败");
    assert!(missing.to_string().contains("container_image"));

    parse_manifest(
        "[plugin.runtime]\nkind='process'\nartifact='dist/plugin.jar'\nhost_version='>=2026.5.10'\ncontainer_image='eclipse-temurin:21-jre@sha256:5c67d24ee8e3dd810b2a0cb6c3827ced2ac5d22729538f90b36c2b9d77678bb8'\nentrypoint=['java','-jar','{artifact}']\nhealth_check='/health'\nshutdown_timeout_seconds=10\n",
    )?;
    let version = parse_manifest(
        "[plugin.runtime]\nkind='process'\nartifact='dist/plugin.js'\nhost_version='soon'\ncontainer_image='node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3'\nentrypoint=['node','{artifact}']\nhealth_check='/health'\n",
    )
    .expect_err("无效版本约束必须失败");
    assert!(version.to_string().contains("版本约束"));
    Ok(())
}

#[test]
fn rejects_floating_process_image_and_undeclared_entrypoint() {
    let floating = parse_manifest(
        "[plugin.runtime]\nkind='process'\nartifact='dist/plugin.js'\ncontainer_image='node:22'\nentrypoint=['node','{artifact}']\nhealth_check='/health'\n",
    )
    .expect_err("浮动镜像必须失败");
    assert!(floating.to_string().contains("sha256"));

    let undeclared = parse_manifest(
        "[plugin.runtime]\nkind='process'\nartifact='dist/plugin.js'\ncontainer_image='node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3'\nentrypoint=['node','src/server.js']\nhealth_check='/health'\n",
    )
    .expect_err("未引用 artifact 的启动命令必须失败");
    assert!(undeclared.to_string().contains("{artifact}"));
}

#[test]
fn enforces_host_version_requirement() -> Result<()> {
    let manifest = parse_manifest(
        "[plugin.runtime]\nkind='page-definition'\nartifact='dist/pages.json'\nhost_version='>=2026.5.10, <2027.0.0'\n",
    )?;
    validate_host_compatibility(&manifest, "2026.9.8")?;
    let error =
        validate_host_compatibility(&manifest, "2027.1.0").expect_err("不匹配的宿主版本必须失败");
    assert!(error.to_string().contains("插件需要宿主版本"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_artifact_symlink_that_leaves_repository() -> Result<()> {
    use std::os::unix::fs::symlink;

    let repository = tempdir()?;
    fs::create_dir(repository.path().join("dist"))?;
    let external = NamedTempFile::new()?;
    symlink(external.path(), repository.path().join("dist/plugin.wasm"))?;
    let error = artifact_path(repository.path(), "dist/plugin.wasm")
        .expect_err("离开仓库的符号链接必须失败");
    assert!(error.to_string().contains("符号链接"));
    Ok(())
}

#[test]
fn validates_process_temporary_storage_budget() -> Result<()> {
    let base = "[plugin.runtime]\nkind='process'\nartifact='server.cjs'\ncontainer_image='node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3'\nentrypoint=['node','{artifact}']\nhealth_check='/health'\n";
    for value in [16, 64, 128] {
        parse_manifest(&format!("{base}temporary_storage_mb={value}\n"))?;
    }
    for value in [0, 15, 129, u64::MAX] {
        assert!(parse_manifest(&format!("{base}temporary_storage_mb={value}\n")).is_err());
    }
    assert!(parse_manifest("[plugin.runtime]\nkind='page-definition'\nartifact='pages.json'\ntemporary_storage_mb=64\n").is_err());
    Ok(())
}
