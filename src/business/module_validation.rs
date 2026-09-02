use std::path::Path;

use anyhow::{Result, ensure};

pub(super) fn validate_module_id(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "业务模块标识不能为空");
    ensure!(
        value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase()),
        "业务模块标识必须以小写字母开头"
    );
    ensure!(
        value.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'),
        "业务模块标识只能包含小写字母、数字和连字符"
    );
    Ok(())
}

pub(super) fn ensure_direct_child(parent: &Path, child: &Path) -> Result<()> {
    ensure!(child.parent() == Some(parent), "业务模块路径越出 lib/biz");
    Ok(())
}

pub(super) fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}
