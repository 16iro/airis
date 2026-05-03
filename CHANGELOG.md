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
- API 키 보관 — `keyring` crate (OS 네이티브 키체인) + `zeroize`
- 6개 Tauri command — `api_key_check`·`api_key_set`·`api_key_delete`·`api_key_present`·`settings_read`·`settings_write`
- API 키 *형식* 검증 (sk-ant- prefix + 길이) — 실제 LLM 호출 검증은 PR 4
- 비밀 키는 절대 JS에 노출 X — `api_key_present`(boolean)만 외부 공개
- `Settings` 타입 — model·language·theme + 원자적 디스크 쓰기 (tmp + rename)
- Zustand `settingsStore` — 메모리 캐시 + 백엔드 동기화
- shadcn 컴포넌트 추가 — Button·Input·Label·Card·Tabs
- `Settings` 페이지 — Tabs 3 섹션 (API 키 / 모델 / 언어)
- `TopBar` + `Mod+,` 단축키로 Settings 토글
- 단위 테스트 +10 (Settings serde 5, 키 형식 검증 5)
- `LlmProvider` trait + `ChatRequest`/`ChatEvent`/`Usage` 타입 (D-005)
- `AnthropicProvider` — `reqwest` + `rustls`, `/v1/messages` POST + 스트리밍
- 직접 SSE 파서 (`SseParser`) — W3C 표준 1층만, 4종 에러 분류 (`[SSE-WIRE]`/`[SSE-EVENT-UNKNOWN]`/`[SSE-PAYLOAD-UNKNOWN]`/`[SSE-JSON]`)
- 백오프 — 429 한정 1s/2s/4s/8s ±20% jitter (8.6 절). 5xx·네트워크는 즉시 에러
- 모르는 SSE 필드(id·retry)·이벤트(`ping`)는 무시 — 통신 규격 forward-compat
- `MockProvider` — 미리 큐잉한 `ChatEvent` 흘려보내는 테스트용
- `chat_send` command — handle 즉시 반환 + `chat:chunk`·`chat:done`·`chat:error` events
- v0.1 가드: `study_slug != "default"` 또는 `context_section_id` 지정 시 `InvalidInput`
- `AppState`에 `current_file: Mutex<Option<String>>` (PR 5 FileViewer가 채움) + `llm: Arc<dyn LlmProvider>` 슬롯
- 단위 테스트 +20 (SSE 파서 10, Anthropic body·delta·usage·error·backoff 9, mock 1)
- `tauri-plugin-dialog` + `@tauri-apps/plugin-dialog` (파일 선택 다이얼로그)
- `commands/file.rs` — `file_open`·`file_close`·`file_current_content` (.md/.txt, 1MB 한도, UTF-8 검증)
- `Settings` 구조체에 `welcome_seen: bool` 추가 (default false)
- `react-markdown` + `remark-gfm` — GFM 마크다운 렌더 (LLM 응답 + 파일 뷰어)
- 마크다운 기본 스타일 — `tokens.css` `.markdown-body` (v0.3 syntax highlighting 후속)
- `react-i18next` + `i18next` — 한국어 번역 파일(`src/locales/ko.json`)·~50개 키
- PR 3 컴포넌트 한국어 문자열 *전체 추출* (Settings·ApiKeyInput·TopBar)
- shadcn `textarea` 컴포넌트 추가
- Zustand stores +3: `uiStore` (page·theme effective)·`fileStore`·`chatStore`
- `Welcome.tsx` — 첫 실행 환영 화면 (welcome_seen=false 시 표시)
- `Workspace.tsx` — FileViewer (좌) + ChatPanel (우) 2-pane
- `FileViewer.tsx` — 파일 다이얼로그·드래그앤드롭·메타·마크다운 렌더
- `ChatPanel.tsx` — 입력·전송·스트리밍 표시·키 보유 가드
- `ChatMessage.tsx` — 사용자/어시스턴트 분기·스트리밍 인디케이터·에러 배너
- `ThemeToggle.tsx` — system/light/dark 순환 + `prefers-color-scheme` listener
- `App.tsx` — 라우팅 (Welcome/Workspace/Settings) + 단축키(`Mod+,`·`Mod+L`·`Mod+Enter`) + drag-drop (`getCurrentWebview().onDragDropEvent`)
- `tests/fixtures/sample.md` — 검증용 샘플 교재
