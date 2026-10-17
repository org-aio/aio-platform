//! 可替换执行器以惰性数据流测试宿主，不依赖某个内置引擎或数据库。
use anyhow::Result;
use az_plugin_contract::transport::Header;
use az_plugin_host::runtime::server::plugins::{ComponentRequest, StreamingResponse};
use futures_util::{StreamExt, stream};

pub(super) fn response(request: ComponentRequest) -> Result<StreamingResponse> {
    assert!(request.body.is_empty());
    assert!(request.headers.is_empty());
    let mode = request.path.rsplit('/').next().unwrap_or_default();
    let size = if mode == "large" {
        100 * 1024 * 1024
    } else {
        64
    };
    let body = if mode == "slow" {
        stream::once(async { Ok(vec![0x5a; 64]) })
            .chain(stream::pending())
            .boxed()
    } else {
        stream::unfold(0, move |offset| async move {
            if offset >= size {
                return None;
            }
            let count = (size - offset).min(64 * 1024);
            Some((Ok(vec![0x5a; count]), offset + count))
        })
        .boxed()
    };
    let mut headers = vec![
        Header {
            name: "content-type".into(),
            value: if mode == "html" {
                "text/html"
            } else {
                "image/png"
            }
            .into(),
        },
        Header {
            name: "set-cookie".into(),
            value: "session=forged".into(),
        },
        Header {
            name: "location".into(),
            value: "https://outside.test".into(),
        },
    ];
    if mode != "slow" {
        headers.push(Header {
            name: "content-length".into(),
            value: if mode == "short" { size + 1 } else { size }.to_string(),
        });
    }
    if mode == "duplicate" {
        headers.push(Header {
            name: "content-length".into(),
            value: size.to_string(),
        });
    }
    Ok(StreamingResponse {
        status: if mode == "redirect" { 302 } else { 200 },
        headers,
        body,
    })
}
