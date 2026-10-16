use crate::{
    bindings::aio::plugin::database::{Rows, Value},
    database_policy::validate,
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::TryStreamExt;
use sqlx::{Column, Postgres, Row, Transaction, TypeInfo, ValueRef, postgres::PgArguments};
const MAX_ROWS: usize = 1_000;
const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
fn bind<'a>(
    statement: &'a str,
    parameters: Vec<Value>,
) -> Result<sqlx::query::Query<'a, Postgres, PgArguments>> {
    validate(statement)?;
    ensure!(parameters.len() <= 256, "SQL 参数超过配额");
    let mut query = sqlx::query(statement);
    for value in parameters {
        query = match value {
            Value::Null => query.bind(crate::database_parameters::InferredNull),
            Value::Boolean(value) => query.bind(value),
            Value::Integer(value) => query.bind(value),
            Value::Real(value) => query.bind(value),
            Value::Text(value) => query.bind(value),
            Value::Bytes(value) => query.bind(value),
            Value::Json(value) => query.bind(sqlx::types::Json(serde_json::from_str::<
                serde_json::Value,
            >(&value)?)),
        };
    }
    Ok(query)
}

pub(crate) async fn execute(
    transaction: &mut Transaction<'static, Postgres>,
    statement: &str,
    parameters: Vec<Value>,
) -> Result<u64> {
    Ok(bind(statement, parameters)?
        .execute(&mut **transaction)
        .await
        .context("执行插件 SQL 失败")?
        .rows_affected())
}

pub(crate) async fn query(
    transaction: &mut Transaction<'static, Postgres>,
    statement: &str,
    parameters: Vec<Value>,
) -> Result<Rows> {
    let mut stream = bind(statement, parameters)?.fetch(&mut **transaction);
    let mut output = Rows {
        columns: Vec::new(),
        values: Vec::new(),
    };
    let mut bytes = 0;
    while let Some(row) = stream.try_next().await.context("读取插件 SQL 结果失败")? {
        ensure!(output.values.len() < MAX_ROWS, "SQL 返回行数超过配额");
        if output.columns.is_empty() {
            output.columns = row
                .columns()
                .iter()
                .map(|column| column.name().to_owned())
                .collect();
        }
        let mut values = Vec::new();
        for (index, column) in row.columns().iter().enumerate() {
            let raw = row.try_get_raw(index)?;
            let value = if raw.is_null() {
                Value::Null
            } else {
                match column.type_info().name() {
                    "BOOL" => Value::Boolean(row.try_get(index)?),
                    "INT2" => Value::Integer(i64::from(row.try_get::<i16, _>(index)?)),
                    "INT4" => Value::Integer(i64::from(row.try_get::<i32, _>(index)?)),
                    "INT8" => Value::Integer(row.try_get(index)?),
                    "FLOAT4" => Value::Real(f64::from(row.try_get::<f32, _>(index)?)),
                    "FLOAT8" => Value::Real(row.try_get(index)?),
                    "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" => Value::Text(row.try_get(index)?),
                    "BYTEA" => Value::Bytes(row.try_get(index)?),
                    "JSON" | "JSONB" => Value::Json(
                        row.try_get::<sqlx::types::Json<serde_json::Value>, _>(index)?
                            .0
                            .to_string(),
                    ),
                    kind => bail!("SQL 返回类型 {kind} 需要显式转换为受支持的 WIT 类型"),
                }
            };
            bytes += match &value {
                Value::Text(value) | Value::Json(value) => value.len(),
                Value::Bytes(value) => value.len(),
                _ => 16,
            };
            ensure!(bytes <= MAX_RESULT_BYTES, "SQL 结果字节数超过配额");
            values.push(value);
        }
        output.values.push(values);
    }
    Ok(output)
}
