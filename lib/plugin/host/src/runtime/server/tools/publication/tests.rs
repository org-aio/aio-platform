use super::{identity::Identity, validation::validate};
use az_tool::publication::Publication;

fn identity() -> Identity {
    Identity {
        repository: "owner/tool".into(),
        repository_owner: "owner".into(),
        sha: "a".repeat(40),
        reference: "refs/heads/main".into(),
        event_name: "push".into(),
        workflow_ref: "owner/tool/.github/workflows/aio-cli.yml@refs/heads/main".into(),
    }
}

#[test]
fn requires_publisher_owner_and_trusted_workflow_source() {
    let valid = identity();
    valid.validate("owner").unwrap();
    assert!(valid.validate("another").is_err());
    for (field, value) in [
        ("repository", "another/tool"),
        ("event_name", "pull_request_target"),
        ("sha", "main"),
        (
            "workflow_ref",
            "owner/tool/.github/workflows/other.yml@refs/heads/main",
        ),
    ] {
        let mut object = serde_json::json!({"repository":valid.repository,"repository_owner":valid.repository_owner,"sha":valid.sha,"ref":valid.reference,"event_name":valid.event_name,"workflow_ref":valid.workflow_ref});
        object[field] = value.into();
        assert!(
            serde_json::from_value::<Identity>(object)
                .unwrap()
                .validate("owner")
                .is_err()
        );
    }
}

#[test]
fn public_package_must_match_signed_source_and_declared_entry() {
    let request = Publication {
        package: "tool".into(),
        version: "0.1.1-dev.7.gabcdef".into(),
    };
    let identity = identity();
    let mut value = serde_json::json!({"name":"tool","version":request.version,"engines":{"node":">=20"},"repository":{"url":"git+https://github.com/owner/tool.git"},"bin":{"tool":"dist/cli.mjs"},"aio":{"cli":{"id":"tool","title":"工具","command":"tool","platforms":["macos","windows","linux"],"setup":["setup"],"uninstall":["restore"]},"source":{"repository":identity.repository,"revision":identity.sha,"reference":identity.reference}},"dist":{"integrity":format!("sha512-{}", "a".repeat(88))}});
    let manifest = validate(
        &request,
        &identity,
        &serde_json::from_value(value.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest.platforms["macos"].install[0].args[2],
        "tool@0.1.1-dev.7.gabcdef"
    );
    assert_eq!(manifest.platforms["macos"].uninstall[0].args, ["restore"]);
    assert_eq!(
        manifest.platforms["macos"].requirements[0]
            .version
            .as_deref(),
        Some(">=20")
    );
    value["aio"]["source"]["revision"] = "b".repeat(40).into();
    assert!(validate(&request, &identity, &serde_json::from_value(value).unwrap()).is_err());
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn keeps_versions_immutable_and_cannot_take_over_another_repository() -> anyhow::Result<()> {
    use az_tool::registration::{Documentation, Metadata};
    use sqlx::{
        PgPool,
        postgres::{PgConnectOptions, PgPoolOptions},
    };
    use std::str::FromStr;
    let url = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = PgPool::connect(&url).await?;
    let schema = format!("cli_publication_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let pool = PgPoolOptions::new()
        .connect_with(PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]))
        .await?;
    let result = async {
        super::super::storage::migrate(&pool).await?;
        let mut manifest = super::super::storage::get(&pool, "codex-model-sync", "0.4.1")
            .await?
            .unwrap();
        manifest.version = "0.4.2-dev.2".into();
        let doc = Documentation {
            metadata: Metadata {
                title: "新版".into(),
                summary: "说明".into(),
                git: manifest.homepage.clone(),
            },
            readme: "新版 README".into(),
            ..Default::default()
        };
        super::storage::publish(&pool, &manifest, &doc, &"a".repeat(40), "sha512-new").await?;
        super::storage::publish(&pool, &manifest, &doc, &"a".repeat(40), "sha512-new").await?;
        assert!(
            super::storage::publish(&pool, &manifest, &doc, &"b".repeat(40), "sha512-other")
                .await
                .is_err()
        );
        let mut old = manifest.clone();
        old.version = "0.4.2-dev.1".into();
        super::storage::publish(
            &pool,
            &old,
            &Documentation::default(),
            &"c".repeat(40),
            "sha512-old",
        )
        .await?;
        assert_eq!(
            super::super::storage::documentation(&pool, &manifest.id)
                .await?
                .unwrap()
                .readme,
            "新版 README"
        );
        assert_eq!(
            serde_json::to_value(super::super::storage::entries(&pool).await?)?[0]["rev"],
            manifest.version
        );
        let mut takeover = manifest;
        takeover.version = "0.4.3".into();
        takeover.homepage = "https://github.com/other/tool".into();
        assert!(
            super::storage::publish(&pool, &takeover, &doc, &"d".repeat(40), "sha512-other")
                .await
                .is_err()
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}
