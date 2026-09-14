use super::*;

const MANIFEST: &str = "[plugin.runtime]\nkind='page-definition'\nartifact='dist/pages.json'\n[plugin.frontend]\npath='dist/web'\n[plugin.marketplace]\ntitle='Screen'\nsummary='Fullstack screen'\nlicense='MIT'\ntags=['test']\n";
const PAGES: &[u8] = br#"[{"id":"screen","label":"Screen","icon":null,"scene":{"id":"community","label":"Community"},"body":{"kind":"frontend","entry":"index.html"}}]"#;

fn files() -> BTreeMap<String, Vec<u8>> {
    [
        ("index.html", b"<!doctype html>".to_vec()),
        ("assets/app.wasm", b"\0asm".to_vec()),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_owned(), bytes))
    .collect()
}

fn package(manifest: &str, files: BTreeMap<String, Vec<u8>>) -> Result<PluginPackage> {
    PluginPackage::new(
        "https://example.com/fullstack.git".to_owned(),
        "1.0.0".to_owned(),
        None,
        manifest.to_owned(),
        PAGES,
        files,
    )
}

#[test]
fn frontend_and_backend_form_one_deterministic_content_version() -> Result<()> {
    let package = package(MANIFEST, files())?;
    let decoded = PluginPackage::decode(&package.encode()?)?;
    assert_eq!(decoded.verify()?.frontend, files());
    assert_eq!(decoded.verify()?.artifact, PAGES);
    assert_eq!(decoded.encode()?, package.encode()?);
    Ok(())
}

#[test]
fn verifies_tree_frontend_entries_without_rewriting_signed_artifact() -> Result<()> {
    let tree = br#"{"id":"business","label":"Business","children":[{"id":"reports","label":"Reports","children":[{"id":"screen","label":"Screen","body":{"kind":"frontend","entry":"index.html"}}]}]}"#;
    let create = |assets| {
        PluginPackage::new(
            "https://example.com/fullstack.git".into(),
            "1.0.0".into(),
            None,
            MANIFEST.into(),
            tree,
            assets,
        )
    };
    let verified = PluginPackage::decode(&create(files())?.encode()?)?.verify()?;
    assert_eq!(verified.artifact, tree);
    let mut missing = files();
    missing.remove("index.html");
    assert!(create(missing).is_err());
    Ok(())
}

#[test]
fn changing_either_side_changes_package_revision_and_tampering_is_rejected() -> Result<()> {
    let original = package(MANIFEST, files())?;
    let mut changed = files();
    changed.insert("assets/app.wasm".to_owned(), b"other frontend".to_vec());
    let replacement = package(MANIFEST, changed)?;
    assert_ne!(original.rev, replacement.rev);
    for (path, bytes) in &replacement.frontend {
        let mut tampered = original.clone();
        if bytes != &original.frontend[path] {
            tampered.frontend.insert(path.clone(), bytes.clone());
            assert!(tampered.verify().is_err());
            let mut tampered = original.clone();
            tampered
                .frontend
                .get_mut(path)
                .expect("资产存在")
                .content_base64 = bytes.content_base64.clone();
            assert!(tampered.verify().is_err());
        }
    }
    let mut missing = original.clone();
    missing.frontend.remove("index.html");
    assert!(missing.verify().is_err());
    Ok(())
}

#[test]
fn rejects_undeclared_missing_colliding_and_traversing_assets() {
    assert!(package(MANIFEST, BTreeMap::new()).is_err());
    assert!(
        package(
            &MANIFEST.replace("[plugin.frontend]\npath='dist/web'\n", ""),
            files()
        )
        .is_err()
    );
    for path in [
        "../secret",
        "a/../../secret",
        "assets",
        "a\\b",
        "a//b",
        "INDEX.html",
        "ASSETS",
    ] {
        let mut files = files();
        files.insert(path.to_owned(), vec![1]);
        assert!(package(MANIFEST, files).is_err(), "{path}");
    }
    assert!(
        package(
            &MANIFEST.replace("dist/pages.json", "dist/web/index.html"),
            files()
        )
        .is_err()
    );
    let mut missing = files();
    missing.remove("index.html");
    assert!(package(MANIFEST, missing).is_err());
}

#[test]
fn enforces_combined_byte_and_file_count_limits() {
    let mut large = files();
    large.insert("assets/large.wasm".to_owned(), vec![0; MAX_BUNDLE_BYTES]);
    assert!(package(MANIFEST, large).is_err());
    let mut files = files();
    for index in 0..MAX_FRONTEND_FILES {
        files.insert(format!("assets/{index}.js"), vec![]);
    }
    assert!(package(MANIFEST, files).is_err());
}

#[test]
fn fullstack_bundle_can_exceed_backend_artifact_limit() -> Result<()> {
    let mut assets = files();
    assets.insert(
        "assets/compose.wasm".to_owned(),
        vec![0; crate::MAX_ARTIFACT_BYTES],
    );
    let package = package(MANIFEST, assets)?;
    assert_eq!(
        package.verify()?.frontend["assets/compose.wasm"].len(),
        crate::MAX_ARTIFACT_BYTES
    );
    let mut oversized = package.clone();
    oversized
        .frontend
        .get_mut("assets/compose.wasm")
        .unwrap()
        .content_base64 = STANDARD.encode(vec![0; MAX_BUNDLE_BYTES]);
    assert!(
        oversized
            .verify_frontend(&az_plugin_manifest::parse_manifest(MANIFEST)?, PAGES.len())
            .is_err()
    );
    Ok(())
}
