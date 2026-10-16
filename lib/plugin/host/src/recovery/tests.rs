use std::{fs, process::Command};

use anyhow::{Context, Result};
use serde_json::json;

use super::*;

#[test]
fn staged_contract_migration_keeps_the_last_confirmed_configuration() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("composition.json");
    std::fs::write(&source, b"original")?;
    let mut journal = ConfigurationJournal::open(&root.path().join("journal"), &source)?;
    let original = serde_json::json!({"schema_version":1,"disabled":true});
    let attempt = journal.begin(configuration_digest(b"original"))?;
    journal.confirm(&attempt, original.clone())?;
    let next = serde_json::json!({"schema_version":2,"disabled":true});
    journal.stage_configuration(&next)?;
    assert_eq!(journal.confirmed().context("缺少成功记录")?.value, original);
    let bytes = read_configuration(&source)?;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes)?, next);
    journal.begin(configuration_digest(&bytes))?;
    drop(journal);
    let mut restarted = ConfigurationJournal::open(&root.path().join("journal"), &source)?;
    assert!(restarted.blocked(&configuration_digest(&bytes)));
    assert!(restarted.begin(configuration_digest(&bytes)).is_err());
    assert_eq!(
        restarted.confirmed().context("缺少成功记录")?.value,
        original
    );
    Ok(())
}

#[test]
fn confirmed_configuration_survives_rejection_interruption_and_explicit_restore() -> Result<()> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join("recovery");
    let source = root.path().join("runtime.json");
    let good = json!({"schema_version":1,"mcp":{"enabled":false}});
    let good_bytes = serde_json::to_vec(&good)?;
    fs::write(&source, &good_bytes)?;
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    let attempt = journal.begin(configuration_digest(&good_bytes))?;
    journal.confirm(&attempt, good.clone())?;
    let rejected = journal.begin(configuration_digest(b"rejected"))?;
    journal.reject(&rejected)?;
    assert!(journal.confirm(&rejected, json!({})).is_err());
    drop(journal);
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    assert!(journal.blocked(&rejected.digest));
    assert_eq!(journal.confirmed().context("缺少成功记录")?.value, good);
    let interrupted = journal.begin(configuration_digest(b"interrupted"))?;
    drop(journal);
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    assert!(journal.blocked(&interrupted.digest));
    assert!(journal.begin(interrupted.digest.clone()).is_err());
    fs::write(&source, b"bad current deployment")?;
    journal.restore()?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&source)?)?,
        good
    );
    assert!(!journal.blocked(&interrupted.digest));
    assert_eq!(
        journal.confirmed().context("缺少恢复记录")?.digest,
        configuration_digest(&fs::read(&source)?)
    );
    Ok(())
}

#[test]
fn explicit_retry_does_not_accept_a_stale_attempt() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut journal =
        ConfigurationJournal::open(&root.path().join("state"), &root.path().join("config"))?;
    let old = journal.begin(configuration_digest(b"same input"))?;
    journal.allow_retry()?;
    let current = journal.begin(old.digest.clone())?;
    assert!(journal.confirm(&old, json!({})).is_err());
    journal.confirm(&current, json!({"accepted":true}))?;
    Ok(())
}

#[test]
fn process_lock_probe() -> Result<()> {
    let Some(root) = std::env::var_os("AIO_JOURNAL_TEST_ROOT") else {
        return Ok(());
    };
    let root = std::path::PathBuf::from(root);
    let opened = ConfigurationJournal::open(&root.join("state"), &root.join("config"));
    assert_eq!(
        opened.is_ok(),
        std::env::var("AIO_JOURNAL_TEST_UNLOCKED").is_ok()
    );
    Ok(())
}

#[test]
fn only_one_process_can_own_the_journal_and_read_only_inspection_remains_available() -> Result<()> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join("state");
    let source = root.path().join("config");
    assert!(ConfigurationJournal::inspect(&directory, &source)?.is_none());
    assert!(!directory.exists());
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    journal.begin(configuration_digest(b"input"))?;
    assert!(ConfigurationJournal::inspect(&directory, &source)?.is_some());
    let probe = |unlocked| -> Result<()> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "recovery::tests::process_lock_probe",
                "--nocapture",
            ])
            .env("AIO_JOURNAL_TEST_ROOT", root.path())
            .env_remove("AIO_JOURNAL_TEST_UNLOCKED");
        if unlocked {
            command.env("AIO_JOURNAL_TEST_UNLOCKED", "1");
        }
        let result = command.output()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    };
    probe(false)?;
    drop(journal);
    probe(true)
}

#[test]
fn failed_durable_write_prevents_further_mutation_without_losing_the_old_snapshot() -> Result<()> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join("state");
    let source = root.path().join("config");
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    let good = journal.begin(configuration_digest(b"good"))?;
    journal.confirm(&good, json!({"old":true}))?;
    fs::rename(
        directory.join("record.json"),
        directory.join("record.backup"),
    )?;
    fs::create_dir(directory.join("record.json"))?;
    assert!(journal.begin(configuration_digest(b"candidate")).is_err());
    assert!(!journal.is_healthy());
    assert!(journal.allow_retry().is_err());
    drop(journal);
    fs::remove_dir(directory.join("record.json"))?;
    fs::rename(
        directory.join("record.backup"),
        directory.join("record.json"),
    )?;
    let journal = ConfigurationJournal::open(&directory, &source)?;
    assert_eq!(
        journal.confirmed().context("旧记录丢失")?.value,
        json!({"old":true})
    );
    Ok(())
}

#[test]
fn corrupt_foreign_or_oversized_records_are_not_silently_treated_as_first_start() -> Result<()> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join("state");
    let source = root.path().join("config");
    let mut journal = ConfigurationJournal::open(&directory, &source)?;
    journal.begin(configuration_digest(b"input"))?;
    drop(journal);
    assert!(ConfigurationJournal::open(&directory, &root.path().join("other-source")).is_err());
    fs::write(directory.join("record.json"), b"{")?;
    assert!(ConfigurationJournal::open(&directory, &source).is_err());
    fs::write(directory.join("record.json"), vec![b' '; 256 * 1024 + 1])?;
    assert!(ConfigurationJournal::inspect(&directory, &source).is_err());
    fs::write(&source, vec![b' '; 64 * 1024 + 1])?;
    assert!(read_configuration(&source).is_err());
    assert!(read_configuration(root.path()).is_err());
    Ok(())
}
