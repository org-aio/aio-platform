use super::*;

#[test]
fn applications_do_not_accept_shell_or_paths() {
    for value in ["Postman", "Google Chrome", "com.postmanlabs.mac", "备忘录"] {
        assert!(validate_application(value).is_ok());
    }
    for value in [
        "",
        "--args",
        "/Applications/Postman.app",
        "Postman; curl evil",
        "$(id)",
        "Postman\nopen Terminal",
    ] {
        assert!(validate_application(value).is_err());
    }
}

#[test]
fn workspace_discovery_intersects_process_grants() -> Result<()> {
    let grants = vec![
        DESKTOP.into(),
        WORKSPACE.into(),
        SSH.into(),
        "shell.execute".into(),
    ];
    assert_eq!(
        requested_capabilities(&grants, "*", "list")?,
        [DESKTOP, WORKSPACE, SSH]
    );
    assert_eq!(
        requested_capabilities(&grants, WORKSPACE, "submit")?,
        [WORKSPACE]
    );
    assert_eq!(requested_capabilities(&grants, SSH, "submit")?, [SSH]);
    assert!(requested_capabilities(&grants, "*", "task").is_err());
    assert!(requested_capabilities(&grants, "*", "cancel").is_err());
    assert!(requested_capabilities(&grants, "shell.execute", "list").is_err());
    assert!(requested_capabilities(&[DESKTOP.into()], WORKSPACE, "submit").is_err());
    assert!(requested_capabilities(&[], "*", "list").is_err());
    let request: Request =
        serde_json::from_value(json!({"tenantId":"test","userId":"owner","operation":"list"}))?;
    assert!(request.capability.is_none());
    Ok(())
}

#[test]
fn desktop_actions_need_an_observation_and_explicit_process_grant() -> Result<()> {
    assert!(requested_capabilities(&[DESKTOP.into()], CONTROL, "submit").is_err());
    assert_eq!(
        requested_capabilities(&[CONTROL.into()], CONTROL, "submit")?,
        [CONTROL]
    );
    Ok(())
}
