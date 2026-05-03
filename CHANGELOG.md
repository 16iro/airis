# Changelog

본 레포의 변경 사항은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/) 형식을 따른다.
버전 번호는 [Semantic Versioning](https://semver.org/lang/ko/)을 따른다.

## [Unreleased]

### Added
- Tauri 2 + React 19 + TypeScript + Vite 스캐폴딩
- Tailwind v4 + shadcn/ui 설정 (`components.json`, `src/lib/utils.ts`)
- Pretendard Variable + Geist Mono 폰트
- 디자인 토큰 — `src/styles/tokens.css` (shadcn 기본 oklch · 라이트/다크)
- 경로 alias `@/*` → `src/*`
- `tests/` 디렉토리 골격
- `AppError` enum + `AppResult<T>` (`#[serde(tag = "kind")]` — TS union과 1:1)
- `tracing` 기반 로깅 — 일별 회전, 14일 보관, dev 빌드는 stderr 동시 출력
- 민감 정보 마스킹 함수 — `mask_api_key`·`mask_path`
- `rusqlite` (bundled) + `schema_version` 기반 마이그레이션 패턴
- v1 마이그레이션: `failed_llm_jobs` 큐 테이블
- `AppState` — `Mutex<Db>` + tracing `WorkerGuard` 보관
- 단위 테스트 14개 (AppError serde, 마이그 idempotent, 마스킹, CHECK 제약)
