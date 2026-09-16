use anyhow::Result;
use az_plugin_bundle::Bundle;
use az_plugin_delivery::BuildJob;
use std::{env, fs, path::Path, process::Command};

pub fn content_type(source: &Path) -> Result<&'static str> {
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(source.join("aio-plugin.toml"))?)?;
    Ok(
        if manifest
            .get("schema_version")
            .and_then(toml::Value::as_integer)
            == Some(2)
        {
            "application/vnd.aio.component+gzip"
        } else {
            "application/vnd.aio.plugin+gzip"
        },
    )
}

pub fn write(source: &Path, archive: &Path, job: &BuildJob) -> Result<()> {
    if content_type(source)? == "application/vnd.aio.component+gzip" {
        let bundle = Bundle::from_directory(
            source,
            "aio-plugin.toml",
            job.git.clone(),
            job.source_revision.clone(),
            job.version.clone(),
        )?;
        let bytes = bundle.encode()?;
        let temporary = archive.with_extension("partial");
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, archive)?;
        Ok(())
    } else {
        crate::build::checked(
            Command::new(env::var("AIO_CLI").unwrap_or_else(|_| "aio".into()))
                .args(["plugin", "package"])
                .arg(source)
                .args(["--git", &job.git, "--version", &job.version, "-o"])
                .arg(archive),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packages_process_artifacts_with_locked_job_identity() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("frontend"))?;
        fs::write(root.path().join("frontend/index.html"), "delivery frontend")?;
        fs::create_dir(root.path().join("migrations"))?;
        fs::write(root.path().join("migrations/001.sql"), "SELECT 1;")?;
        let mut binary = vec![0; 64];
        binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
        binary[18] = 62;
        fs::write(root.path().join("server"), binary)?;
        fs::write(
            root.path().join("aio-plugin.toml"),
            format!(
                "schema_version=2\n[plugin.runtime]\nartifact='server'\nhost_version='>=2026.9.11'\n[plugin.runtime.process]\nimage='sha256:{}'\n[plugin.frontend]\npath='frontend'\n[plugin.database]\nmigrations='migrations'\n[plugin.capabilities]\ndatabase=true\n",
                "a".repeat(64)
            ),
        )?;
        let recipe = az_plugin_delivery::parse(
            "version=1\n[build]\nenvironment='fullstack'\ncommand=['sh','scripts/build.sh']",
        )?
        .build;
        let job = BuildJob {
            id: 42,
            lease: "test".into(),
            git: "https://github.com/example/plugin.git".into(),
            source_revision: "b".repeat(40),
            version: "0.0.0-dev.42".into(),
            recipe,
        };
        let archive = root.path().join("plugin.aio-plugin");
        write(root.path(), &archive, &job)?;
        let bundle = Bundle::decode(&fs::read(archive)?)?;
        assert_eq!(bundle.git, job.git);
        assert_eq!(bundle.commit, job.source_revision);
        assert_eq!(bundle.version, job.version);
        assert_eq!(bundle.files.len(), 3);
        assert_eq!(
            content_type(root.path())?,
            "application/vnd.aio.component+gzip"
        );
        Ok(())
    }
}
