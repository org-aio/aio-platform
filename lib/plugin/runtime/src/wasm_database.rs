use crate::{DatabaseRows, DatabaseValue, bindings::aio::plugin::database};

// 仅 WIT 边界进行类型转换，数据库实现无需编译 Wasmtime。
impl From<database::Value> for DatabaseValue {
    fn from(value: database::Value) -> Self {
        match value {
            database::Value::Null => Self::Null,
            database::Value::Boolean(value) => Self::Boolean(value),
            database::Value::Integer(value) => Self::Integer(value),
            database::Value::Real(value) => Self::Real(value),
            database::Value::Text(value) => Self::Text(value),
            database::Value::Bytes(value) => Self::Bytes(value),
            database::Value::Json(value) => Self::Json(value),
        }
    }
}

impl From<DatabaseValue> for database::Value {
    fn from(value: DatabaseValue) -> Self {
        match value {
            DatabaseValue::Null => Self::Null,
            DatabaseValue::Boolean(value) => Self::Boolean(value),
            DatabaseValue::Integer(value) => Self::Integer(value),
            DatabaseValue::Real(value) => Self::Real(value),
            DatabaseValue::Text(value) => Self::Text(value),
            DatabaseValue::Bytes(value) => Self::Bytes(value),
            DatabaseValue::Json(value) => Self::Json(value),
        }
    }
}

impl From<DatabaseRows> for database::Rows {
    fn from(rows: DatabaseRows) -> Self {
        Self {
            columns: rows.columns,
            values: rows
                .values
                .into_iter()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
        }
    }
}
