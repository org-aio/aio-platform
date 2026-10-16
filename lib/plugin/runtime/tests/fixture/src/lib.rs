wit_bindgen::generate!({
    path: "../../../contract/wit",
    world: "plugin",
});

use aio::plugin::{
    host,
    metadata::{PageDefinition, Scene, Surface},
};

struct Fixture;

impl Guest for Fixture {
    fn describe() -> Description {
        Description {
            label: "Lifecycle fixture".into(),
            pages: vec![PageDefinition {
                id: "fixture".into(),
                label: "Fixture".into(),
                entry: "index.html".into(),
                scene: Some(Scene {
                    id: "test".into(),
                    label: "Test".into(),
                }),
                menu_path: vec![],
                permission: None,
                surface: Surface::Workspace,
            }],
        }
    }

    fn health() -> Result<(), String> {
        if cfg!(feature = "unhealthy") {
            Err("Unhealthy fixture".into())
        } else {
            Ok(())
        }
    }

    fn lifecycle(phase: Phase) -> Result<(), String> {
        let permission = match phase {
            Phase::Prepare => "prepare",
            Phase::Activate => "activate",
            Phase::Deactivate => "deactivate",
        };
        host::authorize(permission).map(|_| ())
    }

    fn handle(request: Request) -> Response {
        if request.path == "/storage" || request.path == "/database" {
            let result = resource_request(&request);
            return match result {
                Ok(body) => Response {
                    status: 200,
                    headers: vec![],
                    body,
                },
                Err(error) => Response {
                    status: 403,
                    headers: vec![],
                    body: error.into_bytes(),
                },
            };
        }
        if request.path == "/management" {
            return aio::plugin::management::invoke(&request).unwrap_or_else(|error| Response {
                status: 403,
                headers: vec![],
                body: error.into_bytes(),
            });
        }
        #[cfg(feature = "enhancements")]
        if request.path == "/enhancement" {
            let result = String::from_utf8(request.body)
                .map_err(|error| error.to_string())
                .and_then(|input| aio::plugin::enhancements::invoke("/v1/math/double", &input));
            return match result {
                Ok(output) => Response {
                    status: 200,
                    headers: vec![],
                    body: output.into_bytes(),
                },
                Err(error) => Response {
                    status: 403,
                    headers: vec![],
                    body: error.into_bytes(),
                },
            };
        }
        if request.path == "/environment" {
            return Response {
                status: 200,
                headers: vec![],
                body: std::env::vars().count().to_string().into_bytes(),
            };
        }
        if request.path == "/wait" {
            let _ = host::authorize("wait");
        }
        Response {
            status: 200,
            headers: vec![],
            body: request.body,
        }
    }
}

// 通过真实 WIT 导入读取宿主替代后端，测试不能绕过能力检查直接访问测试状态。
fn resource_request(request: &Request) -> Result<Vec<u8>, String> {
    if request.path == "/storage" {
        if request.method == "POST" {
            aio::plugin::storage::write("shared", &request.body)?;
        }
        return aio::plugin::storage::read("shared");
    }
    let transaction = aio::plugin::database::begin()?;
    if request.method == "PUT" {
        let value = String::from_utf8(request.body.clone()).map_err(|error| error.to_string())?;
        aio::plugin::database::execute(
            transaction,
            "INSERT INTO records (id,value) VALUES (1,$1) ON CONFLICT(id) DO UPDATE SET value=$1",
            &[aio::plugin::database::Value::Text(value)],
        )?;
    }
    let rows =
        aio::plugin::database::query(transaction, "SELECT value FROM records WHERE id=1", &[])?;
    aio::plugin::database::finish(transaction, true)?;
    match rows.values.first().and_then(|row| row.first()) {
        Some(aio::plugin::database::Value::Text(value)) => Ok(value.as_bytes().to_vec()),
        _ => Err("测试数据库没有预期记录".into()),
    }
}

export!(Fixture);
