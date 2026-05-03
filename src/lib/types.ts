// Rust src-tauri/src/error.rs::AppError 와 1:1 매칭.
// serde(tag = "kind") 직렬화 결과 = 평탄 union.
export type AppError =
  | { kind: "NotFound"; message: string }
  | { kind: "InvalidInput"; message: string }
  | { kind: "LlmApi"; message: string }
  | { kind: "LlmQueued"; job_id: number }
  | { kind: "AuthRequired" }
  | { kind: "NetworkUnavailable" }
  | { kind: "RateLimited"; retry_after_seconds: number }
  | { kind: "Db"; message: string }
  | { kind: "Fs"; message: string }
  | { kind: "Parser"; message: string }
  | { kind: "Internal"; message: string };

export const isAppError = (e: unknown): e is AppError =>
  typeof e === "object" && e !== null && "kind" in e;

// 사용자 노출 메시지로 변환. v0.2에 i18n 키로 매핑 예정.
export const appErrorMessage = (e: AppError): string => {
  switch (e.kind) {
    case "AuthRequired":
      return "API 키가 필요합니다";
    case "NetworkUnavailable":
      return "네트워크 연결을 확인해주세요";
    case "RateLimited":
      return `요청 한도 초과 — ${e.retry_after_seconds}초 후 다시 시도`;
    default:
      return "message" in e ? e.message : "알 수 없는 오류";
  }
};

// 백엔드 src-tauri/src/settings.rs::Settings 와 동일한 모양.
export interface Settings {
  model: string;
  language: string;
  theme: string;
}

export type Provider = "anthropic";

export const DEFAULT_SETTINGS: Settings = {
  model: "claude-opus-4-7",
  language: "ko",
  theme: "system",
};

// v0.1 모델 목록. v0.2부터 동적으로 ProviderCapabilities에서 가져옴.
export const ANTHROPIC_MODELS = [
  { id: "claude-opus-4-7", label: "Claude Opus 4.7 (가장 똑똑, 가장 비쌈)" },
  { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6 (균형)" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5 (빠름·저렴)" },
] as const;
