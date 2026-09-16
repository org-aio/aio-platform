use super::model::{Description, Page};
use anyhow::Result;

#[test]
fn settings_require_an_existing_standalone_page() -> Result<()> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("web"))?;
    std::fs::write(
        root.path().join("web/settings.html"),
        "<html>Settings</html>",
    )?;
    std::fs::write(root.path().join("server"), b"native development server")?;
    std::fs::write(
        root.path().join("aio-plugin.toml"),
        format!(
            "schema_version=2\n[plugin]\nsettings_page='settings'\n[plugin.runtime]\nartifact='server'\nhost_version='>=2026.9.18'\n[plugin.runtime.process]\nimage='sha256:{}'\n[plugin.frontend]\npath='web'\n",
            "a".repeat(64)
        ),
    )?;
    let bundle =
        az_plugin_bundle::VerifiedBundle::from_development_directory(root.path(), "b".repeat(64))?;
    let description = Description {
        process: true,
        label: "测试插件".into(),
        pages: vec![Page {
            id: "settings".into(),
            label: "配置".into(),
            entry: "settings.html".into(),
            scene: None,
            menu_path: vec![],
            permission: Some("settings.use".into()),
            surface: "fullscreen".into(),
        }],
    };
    let classified = description.clone().with_settings(&bundle)?;
    assert_eq!(classified.pages[0].surface, "settings");
    assert_eq!(
        classified.pages[0].permission.as_deref(),
        Some("settings.use")
    );
    let mut missing = description.clone();
    missing.pages.clear();
    assert!(missing.with_settings(&bundle).is_err());
    let mut workspace = description.clone();
    workspace.pages[0].surface = "workspace".into();
    assert!(workspace.with_settings(&bundle).is_err());
    let mut navigation = description;
    navigation.pages[0].menu_path.push("业务".into());
    assert!(navigation.with_settings(&bundle).is_err());
    Ok(())
}
