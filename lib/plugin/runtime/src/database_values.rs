use anyhow::{Result, ensure};

pub(crate) const MAX_ROWS: usize = 1_000;
pub(crate) const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;

/// 执行器与数据库后端共用的值；不依赖 WIT 生成代码或数据库驱动。
#[derive(Clone, Debug, PartialEq)]
pub enum DatabaseValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    Bytes(Vec<u8>),
    Json(String),
}

impl DatabaseValue {
    pub(crate) fn size(&self) -> usize {
        match self {
            Self::Text(value) | Self::Json(value) => value.len(),
            Self::Bytes(value) => value.len(),
            _ => 16,
        }
    }
}

/// 一次受限查询的完整结果；列名与每行的值必须一一对应。
#[derive(Debug, PartialEq)]
pub struct DatabaseRows {
    pub columns: Vec<String>,
    pub values: Vec<Vec<DatabaseValue>>,
}

impl DatabaseRows {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.values.len() <= MAX_ROWS, "SQL 返回行数超过配额");
        let mut bytes = 0usize;
        for column in &self.columns {
            bytes = bytes.saturating_add(column.len());
        }
        for row in &self.values {
            ensure!(row.len() == self.columns.len(), "数据库返回列数不一致");
            for value in row {
                bytes = bytes.saturating_add(value.size());
            }
        }
        ensure!(bytes <= MAX_RESULT_BYTES, "SQL 结果字节数超过配额");
        Ok(())
    }
}
