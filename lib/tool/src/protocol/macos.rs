use super::run;
use anyhow::{Context as _, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const REGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

fn app_path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("无法定位用户目录")?)
            .join("Applications/AIO Helper.app"),
    )
}

fn literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn register(binary: &Path) -> Result<()> {
    let app = app_path()?;
    fs::create_dir_all(app.parent().context("助手目录无效")?)?;
    let root = crate::install::helper_root()?;
    let source = root.join("launcher.applescript");
    let prefix = literal(&format!(
        "{} open ",
        super::shell_quote(binary.to_str().context("助手路径不是 UTF-8")?)
    ));
    // 由系统打开一次性 command 文件，避免申请控制 Terminal 的自动化权限。
    fs::write(
        &source,
        format!(
            r##"on open location targetURL
  set commandText to {prefix} & quoted form of targetURL
  set tempDirectory to do shell script "/usr/bin/mktemp -d /tmp/aio-install.XXXXXXXX"
  set scriptPath to tempDirectory & "/install.command"
  set scriptText to "#!/bin/sh" & linefeed & commandText & linefeed & "aio_status=$?" & linefeed & "/bin/rm -f -- " & quoted form of scriptPath & linefeed & "/bin/rmdir -- " & quoted form of tempDirectory & linefeed & "exit $aio_status" & linefeed
  do shell script "/usr/bin/printf %s " & quoted form of scriptText & " > " & quoted form of scriptPath & " && /bin/chmod 700 " & quoted form of scriptPath
  do shell script "/usr/bin/open -a Terminal " & quoted form of scriptPath
end open location
"##
        ),
    )?;
    let staged = root.join("AIO Helper.app");
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    run(Command::new("osacompile")
        .arg("-o")
        .arg(&staged)
        .arg(&source))?;
    let plist = staged.join("Contents/Info.plist");
    run(Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Add :CFBundleIdentifier string site.addzero.aio.helper",
        ])
        .arg(&plist))?;
    for instruction in [
        "Add :CFBundleURLTypes array",
        "Add :CFBundleURLTypes:0 dict",
        "Add :CFBundleURLTypes:0:CFBundleURLName string AIO",
        "Add :CFBundleURLTypes:0:CFBundleURLSchemes array",
        "Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string aio",
    ] {
        run(Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", instruction])
            .arg(&plist))?;
    }
    // 修改 Info.plist 后必须重新签名，否则 LaunchServices 会拒绝运行 applet。
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&staged))?;
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&staged))?;
    if app.exists() {
        run(Command::new(REGISTER).arg("-u").arg(&app))?;
        fs::remove_dir_all(&app)?;
    }
    fs::rename(staged, &app)?;
    run(Command::new(REGISTER).arg("-f").arg(&app))
}

pub(super) fn unregister() -> Result<()> {
    let app = app_path()?;
    if app.exists() {
        run(Command::new(REGISTER).arg("-u").arg(&app))?;
        fs::remove_dir_all(app)?;
    }
    Ok(())
}
