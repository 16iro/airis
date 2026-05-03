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
- `jobs.rs` — `failed_llm_jobs` 큐 헬퍼 (enqueue_or_update / list_jobs / fetch_payload / delete_job / is_retryable_error)
- 큐 dedup — UNIQUE(study_slug, job_type, payload_json) 충돌 시 attempts++ + error·last_attempt 갱신
- `chat_send`가 NetworkUnavailable·HTTP 5xx·SSE-WIRE 에러 시 자동 큐 적재 (4xx·AuthRequired는 적재 X)
- 새 commands: `retry_failed_job` / `list_failed_jobs` / `delete_failed_job`
- `chat:error` payload에 `job_id` 추가 (큐 적재된 경우)
- `ChatMessage`에 "다시 시도" 버튼 (job_id 보유 시) — 클릭 시 `retry_failed_job` 호출 + 새 어시스턴트 메시지 시작 + 기존 메시지의 job_id 비움
- v0.1 정책: *자동 워커 X* — 사용자 명시 재시도만. 자동 워커는 v0.2 (sequences.md SEQ-6 글자대로엔 못 미침)
- 단위 테스트 +8 (jobs: enqueue·dedup·list·fetch·delete·retryable 분류)
- `.github/workflows/test.yml` — PR / push 트리거 CI
- 3 OS 매트릭스 (ubuntu·macos·windows): `cargo fmt --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test`
- 별도 ubuntu 잡: `cargo audit` (taiki-e/install-action) · TS `pnpm typecheck`·`pnpm build`·`pnpm audit --audit-level=high`
- `Swatinem/rust-cache@v2` 빌드 캐싱
- concurrency 그룹 — 같은 브랜치 push 연달아 시 이전 실행 자동 취소
- v0.2 도입 예정: `vitest` (`pnpm test:unit`)·`eslint` (`pnpm lint`)
- v0.3+ Playwright E2E는 `nightly-e2e.yml`로 분리

### Changed
- `src-tauri/Cargo.lock`을 git 추적 시작 (Tauri 앱 = binary, 재현 빌드 + `cargo audit` 재현성)
- DB 마이그 v2 — `studies`·`chat_messages`·`books` 테이블 추가, `failed_llm_jobs`에 FK + ON DELETE CASCADE 부착 (CREATE+COPY+RENAME 패턴)
- `chat_send`의 `study_slug` 가드 제거 — 활성 스터디 슬러그 그대로 사용 (실존 검증 + chat_messages 영속)
- `studies.is_active` 컬럼 + partial unique index = 활성 스터디 source of truth (메모리 캐시는 `AppState.active_study`)

### Added (v0.2 PR 11)
- DB 마이그 v3 — `paragraphs` (검색 단위, 섹션을 ~500자 청크로 분할) + `paragraphs_fts` (SQLite FTS5 virtual table, unicode61 tokenizer) + 자동 동기화 트리거 (INSERT/UPDATE/DELETE)
- `index/chunker.rs` — 문장 경계 보존 청킹 (한국어 종결·영어 마침표·줄바꿈, hard max 강제 분할)
- `index/keyword.rs` — 트랜잭션 단위 paragraphs rebuild (FTS는 트리거가 자동)
- `commands/book.rs` — `add_main_book`·`add_sub_book`·`list_books`·`remove_book`·`start_indexing`. SHA-256 파일 해싱(sha2 crate). PDF는 PR 12로 이연 (인덱싱 시 안내 에러)
- `commands/search.rs::search_sections` — FTS5 MATCH (prefix 와일드카드 자동) + bm25 점수 + Top-K=5 + snippet 하이라이트
- `chat_send` 컨텍스트 자동 주입 — current_file 본문 우선, 없으면 활성 스터디 책에서 FTS5 검색 → Top-K 섹션 자동 컨텍스트
- 마법사 단계 3 추가 — 완료 안내 (책 등록은 워크스페이스에서)
- `components/AddBookDialog.tsx` — 파일 선택 + 메타 입력 + 등록 + 인덱싱 + 진행률 (`index:progress` event)
- `components/BookList.tsx` — 워크스페이스 상단 책 목록 + "책 추가" 버튼 + 삭제 + indexed 상태 표시
- `bookStore` (Zustand) — books·refresh·add·remove·startIndexing
- 단위 테스트 +16 (chunker 6 + keyword 3 + search 4 + db v3 2 + chunker hard split 1)
- 의존성 추가: `sha2` 0.10
- D-018·D-060 supersede + 새 D-064/D-065 추가 (v0.2 임베딩·하이브리드 미도입, v0.3 검토)

### Added (v0.2 PR 10)
- 책 파서 라이브러리 (`src-tauri/src/parsers/`) — F2 결정적 코어. PR 11 commands에서 호출 들어오면 활성화
- `parsers/types.rs` — `Section`·`SectionLevel`(Chapter/Section)·`ParsedBook`·`BookMetadata`·`BookFormat`. 4계층 모델 (L1 Book / L2 Chapter / L3 Section / L4 Paragraph는 PR 11)
- `parsers/slug.rs` — 챕터 번호 정규식(영문 "Chapter N"·"Ch.N"·한글 "제 N 장"·"N장") + 한글 보존 path 슬러그 + 충돌 시 `-2`·`-3` suffix
- `parsers/markdown.rs` — `pulldown-cmark` 기반 ATX/Setext heading 추적. h1=Chapter, h2~h6=Section. h1 부재 시 첫 h2 챕터 승격. 본문은 heading 사이 raw 마크다운
- `parsers/html.rs` — `ammonia` sanitize → `scraper` heading 추출. script·on* 제거 + strong·em·code 보존. 텍스트 평탄화로 본문 추출
- `parsers/pdf.rs` — `pdfium-render` 기반 페이지 텍스트 추출 + 챕터 정규식 폴백. PDFium binary는 runtime 동적 로드 (앱 번들 동봉)
- 결정 (PR 10): PDF 엔진 = pdfium-render (한국어 정확도 1순위), 섹션 ID = `{book-uuid}/Ch04/§State` 의미 path
- PDF Outline(북마크) 기반 L1 추출은 PR 19로 이연 (pdfium-render 0.8 API 검토 추가 필요)
- 의존성 추가: `pulldown-cmark` 0.12 + `scraper` 0.21 + `ammonia` 4 + `pdfium-render` 0.8
- 단위 테스트 +23 (slug 8: 영/한 챕터·padding·section path·dedupe·display label · markdown 6: h1/h2·한글·h2 승격·dedup·body 추출·empty · html 5: 계층·body·sanitize 2종·empty · pdf 3: chapter 폴백·empty·dedup)

### Added (v0.2 PR 9)
- F1 Library 페이지 (`pages/Library.tsx`) — 카드 그리드, 활성 강조, 정렬(활성 우선·last_opened DESC), 카드 클릭 시 활성 전환 + 워크스페이스 이동
- 새 스터디 마법사 (`pages/NewStudyWizard.tsx`) — 한 화면 + step indicator (옵션 A 결정), 2단계 (이름·슬러그 / stated_goal·deadline). PR 10·11에서 단계 추가 예정
- `components/StepIndicator.tsx` — 진행률 표시. 옵션 B(슬라이드) 도입 시 그대로 재사용
- 삭제 확인 다이얼로그 — 카드별 삭제 + 한 번 더 확인. 백엔드는 삭제 후 다른 스터디로 자동 활성 전환
- TopBar 활성 스터디 라벨 + Library 진입 버튼(`Mod+B`)
- 백엔드 `commands/overview.rs` — Overview.md 영속 (`{data_dir}/studies/{slug}/Overview.md`). frontmatter 파서/빌더 (단순 key:value, 외부 crate X). 원자적 쓰기(`.tmp` + rename)
- 새 commands: `study_overview_read`·`study_overview_write_meta`. `create_study` 시 Overview.md 템플릿 자동 생성 (실패는 비치명)
- `studyStore` 확장 — `list`·`refreshList`·`create`·`remove`. Library에서 사용
- 단위 테스트 +8 (overview: round-trip·unknown 키 무시·인라인 주석·따옴표·디스크 round-trip·patch_meta body 보존·default fallback)

### Added (v0.2 PR 8)
- v2 마이그레이션 SQL — v0.1 사용자의 기존 큐 슬러그를 자동 보존(FK 위반 방지)
- `commands/study.rs` — `list_studies`·`create_study`·`select_study`·`delete_study`·`get_active_study` 5 commands + 슬러그/이름 검증
- `ensure_active_or_bootstrap_default` — 부팅 시 활성 스터디 없으면 'default' 자동 생성·활성화 (v0.1 사용자도 끊김 없이 챗 가능)
- `chat_history` command — 활성 스터디의 최근 메시지 시간순 반환 (cursor 페이징)
- 토큰·모델 메타 영속 (creation_tokens·output_tokens·cache_hit_tokens·model)
- 프론트엔드 `studyStore` (Zustand) — 활성 스터디 캐시 + `select`
- `chatStore.hydrate` — 부팅 시 영속 메시지 복원
- 단위 테스트 +16 (Rust: study slug/name 검증·active uniqueness·bootstrap·cascade·chat_history; vitest: ApiKeyInput·ChatMessage 11개)
- vitest + jsdom + @testing-library/react/jest-dom/user-event 도입 — `pnpm test:unit`
- eslint flat config (typescript-eslint + react-hooks + react-refresh) — `pnpm lint --max-warnings 0`
- `.github/workflows/test.yml` 갱신 — TS 잡에 `pnpm lint`·`pnpm test:unit` 단계 추가
