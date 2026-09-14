use crate::{DatabaseProvisioner, Keyring};
use anyhow::Result;
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn binding_survives_restart_and_rejects_rewritten_history() -> Result<()> {
    let url = std::env::var("AIO_TEST_DATABASE_URL")?;
    let source = uuid::Uuid::new_v4().to_string();
    let keyring = Keyring::new("test".into(), BTreeMap::from([("test".into(), [42; 32])]))?;
    let migrations = vec![(
        "0001.sql".into(),
        "CREATE TABLE records (id BIGINT PRIMARY KEY, value TEXT NOT NULL)".into(),
    )];
    let provisioner = DatabaseProvisioner::connect(&url).await?;
    let database = provisioner
        .install(&source, "test", &migrations, &keyring)
        .await?;
    let mut tx = database.begin().await?;
    sqlx::query("INSERT INTO records VALUES(1,'retained')")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    drop(database);
    drop(provisioner);

    let provisioner = DatabaseProvisioner::connect(&url).await?;
    let database = provisioner
        .install(&source, "test", &migrations, &keyring)
        .await?;
    let mut tx = database.begin().await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM records WHERE id=1")
            .fetch_one(&mut *tx)
            .await?,
        "retained"
    );
    tx.commit().await?;
    let mut upgraded = migrations.clone();
    upgraded.push((
        "0002.sql".into(),
        "CREATE INDEX records_value ON records(value)".into(),
    ));
    provisioner
        .install(&source, "test", &upgraded, &keyring)
        .await?;
    upgraded.push(("0003.sql".into(),
        "ALTER TABLE records ADD COLUMN state TEXT NOT NULL DEFAULT 'pending'; ALTER TABLE records ADD CONSTRAINT state_check CHECK (state IN ('pending'))".into()));
    provisioner
        .install(&source, "test", &upgraded, &keyring)
        .await?;
    upgraded.push(("0004.sql".into(),
        "ALTER TABLE records DROP CONSTRAINT state_check; ALTER TABLE records ADD CONSTRAINT state_check CHECK (state IN ('pending','recorded'))".into()));
    let upgraded_database = provisioner
        .install(&source, "test", &upgraded, &keyring)
        .await?;
    let mut tx = upgraded_database.begin().await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "UPDATE records SET state='recorded' WHERE id=1 RETURNING value"
        )
        .fetch_one(&mut *tx)
        .await?,
        "retained"
    );
    tx.commit().await?;
    let mut invalid = upgraded.clone();
    invalid.push(("0005.sql".into(), "ALTER TABLE records DROP CONSTRAINT state_check; ALTER TABLE records ADD CONSTRAINT state_check CHECK (state='invalid')".into()));
    assert!(
        provisioner
            .install(&source, "test", &invalid, &keyring)
            .await
            .is_err()
    );
    provisioner
        .install(&source, "test", &upgraded, &keyring)
        .await?;
    assert!(
        provisioner
            .install(&source, "test", &migrations, &keyring)
            .await
            .is_err()
    );
    upgraded[0].1.push(';');
    assert!(
        provisioner
            .install(&source, "test", &upgraded, &keyring)
            .await
            .is_err()
    );
    upgraded[0] = migrations[0].clone();
    let wrong_key = Keyring::new("test".into(), BTreeMap::from([("test".into(), [43; 32])]))?;
    assert!(
        provisioner
            .install(&source, "test", &upgraded, &wrong_key)
            .await
            .is_err()
    );
    let other = provisioner
        .install(&source, "other", &migrations, &keyring)
        .await?;
    let mut tx = other.begin().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM records")
            .fetch_one(&mut *tx)
            .await?,
        0
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要同一实例中的两个独立测试数据库"]
async fn databases_isolate_roles_and_reopen_persisted_bindings() -> Result<()> {
    let first = DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let second =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_SECOND_DATABASE_URL")?).await?;
    let source = uuid::Uuid::new_v4().to_string();
    let tenant = "database-isolation";
    let keyring = Keyring::new("test".into(), BTreeMap::from([("test".into(), [42; 32])]))?;
    let migrations = vec![(
        "0001.sql".into(),
        "CREATE TABLE records (id BIGINT PRIMARY KEY, value TEXT NOT NULL)".into(),
    )];
    let database = first
        .install(&source, tenant, &migrations, &keyring)
        .await?;
    let mut tx = database.begin().await?;
    sqlx::query("INSERT INTO records VALUES (1,'first-database')")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    drop(database);
    let other = second
        .install(&source, tenant, &migrations, &keyring)
        .await?;
    let mut tx = other.begin().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM records")
            .fetch_one(&mut *tx)
            .await?,
        0
    );
    tx.commit().await?;
    let first_role = first
        .process_connection(&source, tenant, &keyring)
        .await?
        .get_username()
        .to_owned();
    let second_role = second
        .process_connection(&source, tenant, &keyring)
        .await?
        .get_username()
        .to_owned();
    assert_ne!(first_role, second_role);
    // 持久记录拥有角色身份；算法调整或恢复备份不会要求在线服务更换数据库账号。
    let existing_role = format!("r_{}", crate::provision::namespace(&source, tenant));
    for statement in [
        format!("ALTER ROLE {first_role} RENAME TO {existing_role}"),
        format!(
            "ALTER ROLE o_{} RENAME TO o_{}",
            &first_role[2..],
            &existing_role[2..]
        ),
    ] {
        sqlx::query(&statement).execute(&first.pool).await?;
    }
    sqlx::query("UPDATE aio_plugin_host.database_bindings SET role_name=$1 WHERE source_id=$2 AND tenant_id=$3")
        .bind(&existing_role).bind(&source).bind(tenant).execute(&first.pool).await?;
    let reopened = first
        .install(&source, tenant, &migrations, &keyring)
        .await?;
    let mut tx = reopened.begin().await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM records WHERE id=1")
            .fetch_one(&mut *tx)
            .await?,
        "first-database"
    );
    tx.commit().await?;
    assert_eq!(
        first
            .process_connection(&source, tenant, &keyring)
            .await?
            .get_username(),
        existing_role
    );
    Ok(())
}
