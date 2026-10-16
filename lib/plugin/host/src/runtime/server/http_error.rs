use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// 可替换宿主服务在 HTTP 边界返回的错误；状态由构造方法明确选择。
pub struct RuntimeError {
    status: StatusCode,
    error: anyhow::Error,
}

impl RuntimeError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error: anyhow::anyhow!(message.into()),
        }
    }

    pub fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            error: anyhow::anyhow!(message.into()),
        }
    }
}

impl<E> From<E> for RuntimeError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: value.into(),
        }
    }
}

impl IntoResponse for RuntimeError {
    fn into_response(self) -> Response {
        let message = format!("{:#}", self.error);
        (self.status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
