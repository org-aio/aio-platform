use super::revision_matches;

#[test]
fn published_revision_matches_when_install_omits_revision() {
    assert!(revision_matches(&"a".repeat(40), None));
}

#[test]
fn published_revision_matches_only_the_requested_commit() {
    let published = "a".repeat(40);

    assert!(revision_matches(&published, Some(&published)));
    assert!(revision_matches(
        &published,
        Some(&published.to_uppercase())
    ));
    assert!(!revision_matches(&published, Some(&"b".repeat(40))));
}
