// 앱 전역 에러. 모든 Tauri command는 AppResult<T>를 반환한다.
// 직렬화 형식은 src/lib/types.ts 의 AppError union과 1:1 대응된다.
// (참조: design/architecture/api-contract.md "공통 타입" 절)
//
// `#[serde(tag = "kind")]` 사용 이유:
//   thiserror는 Display(`{message}`)에 필드명을 그대로 쓸 수 있고,
//   serde는 named-field variant를 `{kind, ...fields}` 평탄 객체로 직렬화한다.
//   덕분에 TS 쪽 union(`{ kind: 'NotFound'; message: string }`)과 동일한 모양이 된다.

use serde::Serialize;

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(tag = "kind")]
pub enum AppError {
    #[error("not found: {message}")]
    NotFound { message: String },

    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    #[error("LLM API error: {message}")]
    LlmApi { message: String },

    #[error("LLM unavailable (queued: job {job_id})")]
    LlmQueued { job_id: i64 },

    #[error("authentication required")]
    AuthRequired,

    #[error("network unavailable")]
    NetworkUnavailable,

    #[error("rate limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("database error: {message}")]
    Db { message: String },

    #[error("file system error: {message}")]
    Fs { message: String },

    #[error("parser error: {message}")]
    Parser { message: String },

    #[error("internal: {message}")]
    Internal { message: String },
}

pub type AppResult<T> = Result<T, AppError>;

// 외부 에러 → AppError 변환. From 구현으로 `?` 연산자가 자연스럽게 동작.
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db {
            message: e.to_string(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Fs {
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_named_variant_with_kind_tag() {
        let err = AppError::NotFound {
            message: "study x".into(),
        };
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v, json!({ "kind": "NotFound", "message": "study x" }));
    }

    #[test]
    fn serializes_unit_variant_with_only_kind() {
        let err = AppError::AuthRequired;
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v, json!({ "kind": "AuthRequired" }));
    }

    #[test]
    fn serializes_typed_fields_as_numbers() {
        let queued = AppError::LlmQueued { job_id: 42 };
        let limited = AppError::RateLimited {
            retry_after_seconds: 30,
        };
        assert_eq!(
            serde_json::to_value(&queued).unwrap(),
            json!({ "kind": "LlmQueued", "job_id": 42 })
        );
        assert_eq!(
            serde_json::to_value(&limited).unwrap(),
            json!({ "kind": "RateLimited", "retry_after_seconds": 30 })
        );
    }

    #[test]
    fn rusqlite_error_maps_to_db_variant() {
        let err: AppError = rusqlite::Error::QueryReturnedNoRows.into();
        assert!(matches!(err, AppError::Db { .. }));
    }

    #[test]
    fn io_error_maps_to_fs_variant() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: AppError = io_err.into();
        assert!(matches!(err, AppError::Fs { .. }));
    }

    #[test]
    fn display_uses_thiserror_format_string() {
        let err = AppError::RateLimited {
            retry_after_seconds: 30,
        };
        assert_eq!(err.to_string(), "rate limited; retry after 30s");
    }
}
