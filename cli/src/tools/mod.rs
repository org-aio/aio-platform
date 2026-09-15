use anyhow::{Result, bail};
use az_tool::{
    InstallLink,
    install::{self, Store},
};

pub(super) fn run(arguments: &[String]) -> Result<()> {
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    match arguments.as_slice() {
        ["helper", "install"] => az_tool::protocol::register(&std::env::current_exe()?),
        ["helper", "uninstall"] => az_tool::protocol::unregister(),
        ["tool", "install", id, "--version", version] => install_link(InstallLink::parse(
            &format!("aio://install/{id}?version={version}"),
        )?),
        ["open", url] => {
            let result = InstallLink::parse(url).and_then(install_link);
            if let Err(error) = &result {
                eprintln!("操作失败：{error:#}");
            }
            println!("按回车关闭本次操作。");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            result
        }
        ["tool", "uninstall", id] => {
            let store = Store::user()?;
            let Some(record) = store.read(id)? else {
                println!("没有 {id} 的本机安装记录");
                return Ok(());
            };
            if install::confirm(&record.manifest, true)? {
                store.uninstall(id)?;
            }
            Ok(())
        }
        ["tool", "list"] => {
            for item in Store::user()?.list()? {
                println!(
                    "{} {} {}",
                    item.manifest.id, item.manifest.version, item.state
                );
            }
            Ok(())
        }
        _ => bail!(
            "用法：aio helper install|uninstall；aio tool install <id> --version <版本>；aio tool uninstall <id>；aio tool list"
        ),
    }
}

fn install_link(link: InstallLink) -> Result<()> {
    let manifest = install::fetch(&link)?;
    if install::confirm(&manifest, false)? {
        Store::user()?.install(manifest)?;
    }
    Ok(())
}
