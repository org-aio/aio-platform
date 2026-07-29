use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentShape {
    Leaf,
    Container,
    Dual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentPropertyKind {
    Text,
    Boolean,
    Number,
    Choice,
    Action,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentBehavior {
    #[default]
    Generic,
    Button,
    Input,
    Progress,
    Table,
}

impl ComponentBehavior {
    pub fn is_generic(value: &Self) -> bool {
        *value == Self::Generic
    }
}
