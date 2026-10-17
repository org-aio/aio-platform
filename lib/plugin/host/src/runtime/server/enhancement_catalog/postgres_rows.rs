//! 行解码只属于默认 PostgreSQL 实现，公开契约不依赖 SQLx 类型。
use sqlx::{FromRow, Row, postgres::PgRow};

use super::{Event, Head, Observation, Revision};

macro_rules! postgres_row {
    ($model:ty, $($field:ident),+ $(,)?) => {
        impl<'r> FromRow<'r, PgRow> for $model {
            fn from_row(row: &'r PgRow) -> Result<Self, sqlx::Error> {
                Ok(Self { $($field: row.try_get(stringify!($field))?,)+ })
            }
        }
    };
}

postgres_row!(Head, revision, generation);
postgres_row!(Revision, revision, author, created_at);
postgres_row!(Event, generation, revision, actor, action, created_at);
postgres_row!(Observation, path, started, succeeded, failed, updated_at);
