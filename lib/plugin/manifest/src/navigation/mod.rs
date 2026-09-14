mod model;
#[cfg(feature = "validation")]
mod parse;

pub use model::{MenuBranch, MenuNode, MenuPage, PageDocument, SceneTree};
#[cfg(feature = "validation")]
pub use parse::parse_page_definitions;

#[cfg(all(test, feature = "validation"))]
mod tests;
