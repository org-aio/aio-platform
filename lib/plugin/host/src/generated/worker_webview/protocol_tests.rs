use super::*;
use crate::generated::worker_webview::model::WebviewDocument;

struct Application(Vec<WebviewRuntime>);
struct OtherApplication(Vec<WebviewRuntime>);

impl WebviewProtocolProvider for Application {
    fn protocols(&self) -> Vec<WebviewRuntime> {
        self.0.clone()
    }
}
impl WebviewProtocolProvider for OtherApplication {
    fn protocols(&self) -> Vec<WebviewRuntime> {
        self.0.clone()
    }
}

fn declaration(protocol: &str, capability: &str, browser: bool) -> WebviewRuntime {
    WebviewRuntime {
        protocol: protocol.into(),
        capability: capability.into(),
        headless_capability: None,
        document: browser.then_some(WebviewDocument { allow_eval: false }),
    }
}

#[test]
fn independent_protocols_require_explicit_selection_and_do_not_imply_browser_access() -> Result<()>
{
    let viewer = declaration("drawing", "drawing.view", true);
    let stream = declaration("sensor", "sensor.stream", false);
    let protocols = WebviewProtocols::new(vec![Arc::new(Application(vec![
        viewer.clone(),
        stream.clone(),
    ]))]);
    protocols.validate()?;
    assert_eq!(protocols.runtime("drawing")?, viewer);
    assert!(protocols.runtime("unknown").is_err());
    assert!(protocols.runtime("").is_err());
    assert!(protocols.enabled(std::slice::from_ref(&stream.capability))?);
    assert!(protocols.browser_runtime(&[stream.capability])?.is_none());
    assert_eq!(
        protocols.browser_runtime(std::slice::from_ref(&viewer.capability))?,
        Some(viewer)
    );
    assert!(WebviewProtocols::new(vec![]).capabilities()?.is_empty());
    Ok(())
}

#[test]
fn duplicate_types_protocols_and_overlapping_permissions_are_rejected() {
    let viewer = declaration("drawing", "drawing.view", true);
    let repeated = WebviewProtocols::new(vec![
        Arc::new(Application(vec![viewer.clone()])),
        Arc::new(Application(vec![declaration("other", "other.view", true)])),
    ]);
    assert!(repeated.validate().is_err());
    for conflict in [
        declaration("drawing", "other.view", true),
        declaration("other", "drawing.view", true),
        WebviewRuntime {
            headless_capability: Some("drawing.view".into()),
            ..declaration("other", "other.view", true)
        },
    ] {
        let protocols = WebviewProtocols::new(vec![
            Arc::new(Application(vec![viewer.clone()])),
            Arc::new(OtherApplication(vec![conflict])),
        ]);
        assert!(protocols.validate().is_err());
    }
}

#[test]
fn malformed_and_ambiguous_browser_contracts_fail_closed() -> Result<()> {
    for invalid in ["", "Other", "other*", "other/view", "other view"] {
        let protocols = WebviewProtocols::new(vec![Arc::new(Application(vec![declaration(
            invalid,
            "drawing.view",
            true,
        )]))]);
        assert!(protocols.validate().is_err());
    }
    let protocols = WebviewProtocols::new(vec![Arc::new(Application(vec![
        declaration("drawing", "drawing.view", true),
        declaration("other", "other.view", true),
    ]))]);
    protocols.validate()?;
    assert!(
        protocols
            .browser_runtime(&["drawing.view".into(), "other.view".into()])
            .is_err()
    );
    assert!(
        serde_json::from_value::<super::super::model::Access>(serde_json::json!({"enabled":true}))
            .is_err()
    );
    Ok(())
}
