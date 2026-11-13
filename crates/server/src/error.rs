use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub struct Error(pub StatusCode, pub String);
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(status: u16, msg: impl Into<String>) -> Self {
        Self(
            StatusCode::from_u16(status).expect("valid status"),
            msg.into(),
        )
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1)
    }
}
impl std::error::Error for Error {}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        tracing::error!(error=%e, "database operation failed");
        Self::new(500, "database operation failed")
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        tracing::error!(error=%e, "filesystem operation failed");
        Self::new(500, "filesystem operation failed")
    }
}
