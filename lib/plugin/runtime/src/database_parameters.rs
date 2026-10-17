use sqlx::{
    Encode, Postgres, Type,
    encode::IsNull,
    error::BoxDynError,
    postgres::{PgArgumentBuffer, PgTypeInfo, types::Oid},
};

pub(crate) struct InferredNull;

impl Type<Postgres> for InferredNull {
    fn type_info() -> PgTypeInfo {
        // PostgreSQL Parse 协议中 OID 0 表示由 SQL 上下文推导类型。
        PgTypeInfo::with_oid(Oid(0))
    }
}

impl Encode<'_, Postgres> for InferredNull {
    fn encode_by_ref(&self, _: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        Ok(IsNull::Yes)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::{DatabaseProvisioner, DatabaseValue as Value};

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database"]
    async fn null_parameters_infer_all_supported_column_types() -> Result<()> {
        let source = format!(
            "null-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let database = DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?)
            .await?
            .create(
                &source,
                "test",
                "CREATE TABLE records (flag BOOLEAN, number BIGINT, fraction DOUBLE PRECISION, label TEXT, payload BYTEA, document JSONB)",
            )
            .await?;
        let mut transaction = database.begin().await?;
        assert_eq!(
            transaction
                .execute(
                    "INSERT INTO records VALUES ($1, $2, $3, $4, $5, $6)",
                    vec![Value::Null; 6],
                )
                .await?,
            1
        );
        let rows = transaction.query("SELECT * FROM records", vec![]).await?;
        assert_eq!(rows.values.len(), 1);
        assert_eq!(rows.values[0].len(), 6);
        assert!(
            rows.values[0]
                .iter()
                .all(|value| matches!(value, Value::Null))
        );
        let typed = transaction
            .query(
                "SELECT $1::bigint, $2::boolean, $3::bytea",
                vec![Value::Null; 3],
            )
            .await?;
        assert!(
            typed.values[0]
                .iter()
                .all(|value| matches!(value, Value::Null))
        );
        transaction.rollback().await?;
        Ok(())
    }
}
