use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub(super) struct Options {
    pub root: PathBuf,
    pub overrides: Vec<PathBuf>,
    pub debug: bool,
    pub offline: bool,
    pub watch: bool,
    pub open: bool,
    pub port: u16,
    pub database: Option<String>,
    pub jvm_args: Vec<String>,
}

pub(super) fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options {
        root: PathBuf::from("."),
        overrides: vec![],
        debug: false,
        offline: false,
        watch: true,
        open: true,
        port: 0,
        database: std::env::var("AIO_DEV_DATABASE_URL").ok(),
        jvm_args: vec![],
    };
    let mut args = args.iter();
    let mut root = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--debug" => options.debug = true,
            "--offline" => options.offline = true,
            "--no-watch" => options.watch = false,
            "--no-open" => options.open = false,
            "--with" => options
                .overrides
                .push(args.next().context("--with 缺少路径")?.into()),
            "--port" => options.port = args.next().context("--port 缺少端口")?.parse()?,
            "--database-url" => {
                options.database = Some(args.next().context("--database-url 缺少地址")?.clone())
            }
            "--jvm-args" => options
                .jvm_args
                .push(args.next().context("--jvm-args 缺少参数")?.clone()),
            value if !value.starts_with('-') && !root => {
                options.root = value.into();
                root = true;
            }
            other => bail!("未知开发选项: {other}"),
        }
    }
    options.root = options.root.canonicalize()?;
    options.overrides = options
        .overrides
        .into_iter()
        .map(|path| path.canonicalize().map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    Ok(options)
}
