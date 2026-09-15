use serde::Serialize;

#[derive(Serialize)]
#[serde(untagged)]
pub(in crate::runtime::server) enum MarketplaceItem {
    Plugin(crate::runtime::MarketplaceEntry),
    Cli(CliEntry),
}

#[derive(Serialize)]
pub(in crate::runtime::server) struct CliEntry {
    pub git: String,
    pub rev: String,
    pub title: String,
    pub summary: String,
    pub license: String,
    pub tags: Vec<String>,
    pub installed: bool,
    pub cli: az_tool::ToolManifest,
}

impl From<az_tool::ToolManifest> for MarketplaceItem {
    fn from(cli: az_tool::ToolManifest) -> Self {
        Self::Cli(CliEntry {
            git: format!("aio-tool:{}", cli.id),
            rev: cli.version.clone(),
            title: cli.title.clone(),
            summary: cli.summary.clone(),
            license: cli.license.clone(),
            tags: cli.tags.clone(),
            installed: false,
            cli,
        })
    }
}
