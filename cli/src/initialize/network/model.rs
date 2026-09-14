use anyhow::{Result, bail};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NetworkProfile {
    #[default]
    China,
    Global,
}

impl NetworkProfile {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "china" => Ok(Self::China),
            "global" => Ok(Self::Global),
            _ => bail!("--network 只支持 china 或 global"),
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::China => "china",
            Self::Global => "global",
        }
    }
}
