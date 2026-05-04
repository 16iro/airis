# Changelog

본 레포의 변경 사항은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/) 형식을 따른다.
버전 번호는 [Semantic Versioning](https://semver.org/lang/ko/)을 따른다.

## [Unreleased]

### Changed (PR 29 — v0.3 트랙 A: UI 텍스트 sweep)
- ko.json 전수 재작성 — 종결 어미 합니다체 일관 (D-068)
- 마크다운 문법(`*X*`), em dash(`—`), 본문 내 중점(`·`) UI 노출 제거
- 내부 메타 제거 — `v0.x`, `PR NN`, 내부 문서 참조(`release-pipeline.md`)
- 전문 용어 풀어쓰기 — `메타인지 제동`, `정규식 거짓 양성`, `Memory` UI 노출은 `학습 기록`으로
- placeholder 일반화 — `Rust 깊게 보기`, `rust-deep-dive`, `Programming Rust`, `Jim Blandy`, `Ch04` 같은 사례 placeholder 제거
- "회상 챌린지" → "회상 연습", "SRS 복습" → "복습", `Memory` UI 라벨 → "학습 기록"
- BookViewer PDF 에러·페이지 컨트롤 aria-label을 inline에서 i18n 키(`bookviewer.pdf_*`)로 분리
- `decision-log.md` D-067 (v0.3 슬라이스 정의) + D-068 (종결 어미 정책) 추가

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

### Fixed (v0.2.1 PR 28.1 hotfix) — ChatPanel가 auth_mode 무시하고 API 키 강제
- 증상: CLI 모드 전환 + CLI 설치/로그인 완료한 사용자가 챗 화면에서 "API 키 필요" UI를 보고 send 버튼이 작동 안 함
- 원인: `ChatPanel.tsx:78`이 `auth_mode` 무시하고 무조건 `apiKeyPresent(activeProvider)` 호출. CLI 모드에선 keyring에 키가 없으니 `hasKey=false`로 떨어져 UI가 "API 키 입력" 화면으로 박힘
- 수정: `auth_mode === "cli"`일 때 keyring 체크 건너뛰고 `hasKey=true`로 처리. CLI 인증 상태 검증은 백엔드 chat_send에 위임

### Fixed (v0.2.1 PR 28 hotfix) — CLI 모드 전환 시 provider rebuild 누락
- 증상: Welcome → Claude 카드 클릭 → CLI 설치/로그인 완료 → 챗 시도 → "API 키 연결 필요" 에러
- 원인: `settings_write`가 auth_mode=cli로 갱신할 때 *그 시점엔 아직 CLI 미설치*라 `build_provider`가 `Err(CliMissing)`을 반환 → settings는 저장됐지만 `AppState.llm`은 옛날 `AnthropicProvider` 그대로 박힘. 이후 `cli_install_provider`/`cli_login`이 성공해도 누구도 provider rebuild 안 함.
- 수정:
  - `lib.rs::try_rebuild_llm(&state)` 신규 헬퍼 — 현재 settings 기준으로 build 시도, 실패 시 기존 provider 유지(fail-soft)
  - `settings_write`: build_provider 실패해도 에러 안 던짐 — settings 저장은 성공
  - `cli_install_provider`: 설치 성공 후 `try_rebuild_llm` 호출
  - `cli_login`: Anthropic/Codex 로그인 성공 후 `try_rebuild_llm` 호출
  - `cli_auth_status_claude`/`cli_auth_status_gemini`/`cli_auth_status_codex`: `logged_in=true`면 `try_rebuild_llm` 호출 (외부 터미널 인증 케이스 회복)
  - `build_provider` 자체도 fail-soft: CLI build 실패하면 ApiKey 어댑터로 fallback (앱 startup 보장)
- 영향: PR 28 적용 전 사용자는 앱 재시작 또는 Settings → CLI 연결 다이얼로그 재진입으로 recovery 가능

### Added (v0.2.1 PR 27) — 첫 실행 onboarding 재작성 + Settings Advanced 탭
- `Welcome.tsx` 전면 재작성 — "이미 구독 중이세요?" 섹션을 1순위로 노출. Claude(추천)/Gemini(무료)/Codex 카드 클릭 시 `auth_mode=cli` + `active_provider` 저장 후 `CliSetupDialog` 띄움. onComplete 시 자동으로 `welcome_seen=true` + 워크스페이스 이동.
- "구독 없이 API 키로 직접 시작 (Advanced)" 링크 — 클릭 시 `auth_mode=api_key` 설정 후 Settings로 이동
- Settings에 새 "Advanced" 탭 추가 — API 키 직접 입력 카드 이동 (이전엔 Provider 탭 하단에 있었음)
- Provider 탭은 이제 인증 방식(auth_mode) + 프로바이더 선택만 — 깔끔한 의도 분리
- 신규 locale 키 — `welcome.cli.{section_title,section_desc,*_title,*_sub,*_badge}`, `welcome.advanced_link`, `settings.tabs.advanced`, `settings.advanced.api_key_desc`
- 결정 (PR 27): #1 Welcome은 "구독 연결" 중심 — 무구독자는 Gemini 무료 티어 카드로 시작 / #2 API 키 입력은 Advanced 탭으로 강등하되 *제거 X* (사용자 선택권 보장) / #3 신규 사용자 기본 auth_mode는 ApiKey (settings.json 부재 시 default) — Welcome에서 *명시적으로* CLI 선택해야 전환

### Added (v0.2.1 PR 26) — Codex CLI 브릿지
- `llm/codex_cli.rs` — `codex exec --json --model <m> "<query>"` 자식 프로세스 어댑터
- JSONL 파서 — `item.completed{item:{type:"agent_message",text}}` → `ChatEvent::TextDelta`, `turn.completed{usage:{input_tokens,cached_input_tokens,output_tokens,reasoning_output_tokens}}` → `ChatEvent::Done`, `turn.failed`/`error` → `AppError::CliRuntime`
- `agent_reasoning`·`command_execution`·`plan_update` 등 다른 item.type은 무시 (LLM 텍스트 응답만 사용)
- `cli_auth_status_codex` 커맨드 — `codex login status` exit code (0=인증) 활용
- `cli_login` Codex 분기 — `codex login` 직접 spawn (브라우저 OAuth) / `codex login --with-api-key` (console 모드)
- 시스템 프롬프트는 user 본문 앞에 prepend (Gemini와 동일 패턴)
- `build_provider`에 OpenAI → `CodexCliProvider` 분기 활성화 (PR 24/25 인프라 그대로 재사용)
- 프론트 — `CliSetupDialog`의 openai 분기 활성화, `cliAuthStatusCodex` API 추가
- 단위 테스트 +8 (agent_message·reasoning skip·command_execution skip·turn.completed·turn.failed·thread.started·turn.started·malformed)
- 결정 (PR 26): #1 Codex login은 직접 spawn(브라우저 OAuth) 가능 — Gemini와 달리 TerminalInstruction 필요 없음 / #2 agent_message만 통과 (reasoning/command은 chat UI에 노이즈) / #3 cached_input_tokens → cache_read_input_tokens 매핑

### Added (v0.2.1 PR 25) — Gemini CLI 브릿지
- `llm/gemini_cli.rs` — `gemini "<query>" -o stream-json -m <model>` 자식 프로세스 어댑터
- stream-json 라인 파서 — `message{role:"assistant",content,delta:true}` → `ChatEvent::TextDelta` (진짜 델타·차분 계산 X), `result{status,stats:{input_tokens,output_tokens,cached}}` → `ChatEvent::Done`
- 시스템 프롬프트는 user 본문 앞에 `<sys>\n\n---\n\n<query>` 형태로 prepend (CLI 자체 시스템 옵션 부재 회피)
- `cli_auth_status_gemini` 커맨드 — 별도 status 명령 부재 → 짧은 ping(`gemini . -o json -m flash`) exit code로 인증 추정
- `cli_login` Gemini 분기 — 비대화형 login 명령이 마땅치 않아 `CliLoginOutcome::TerminalInstruction { command, hint }` 반환
- 프론트 — `CliSetupDialog` 일반화: anthropic/gemini/openai 분기 + TerminalInstruction 박스 + `recheck` 버튼
- `build_provider`에 Gemini → `GeminiCliProvider` 분기 추가, `locate_required` 헬퍼로 중복 제거
- 단위 테스트 +6 (assistant 델타·user 메시지 skip·result success/failure·init skip·malformed JSON skip)
- 결정 (PR 25): #1 Gemini auth status는 ping으로 추정 — `~/.gemini/oauth_creds.json` 직접 검사 회피 / #2 시스템 프롬프트는 prepend (CLI에 명시 옵션 없음) / #3 비대화형 login 부재 → 사용자 터미널 안내로 우회

### Added (v0.2.1 PR 24) — CLI 인프라 + Claude Code 브릿지
- D-066 결정 — v0.2.1 인증 경로: 공식 CLI subprocess가 메인, API 키 직접 입력은 Advanced 백업 (구독 그대로 활용 박탈감 해소)
- `runtime.rs` — Node/npm PATH 감지 + `~/.airis/npm` 전용 prefix (sudo 회피)
- `cli_install.rs` — `npm install -g --prefix=<airis>` 래퍼 + 프로바이더↔패키지 매핑 (`@anthropic-ai/claude-code`·`@google/gemini-cli`·`@openai/codex`)
- `llm/claude_cli.rs` — Claude Code 자식 프로세스 어댑터: `claude -p ... --output-format stream-json --verbose --no-session-persistence --tools "" --setting-sources ""` + cwd를 app_data_dir로 격리 (사용자 CLAUDE.md 자동 발견 차단)
- stream-json JSONL 파서 — `assistant` 이벤트 누적 차분 → `ChatEvent::TextDelta`, `result` → `ChatEvent::Done` (usage 매핑)
- `tokio::process::Command` + `kill_on_drop(true)` + ChildGuard로 좀비 프로세스 방지
- 신규 Tauri 커맨드 5종 — `cli_runtime_detect`·`cli_status`·`cli_install_provider`·`cli_auth_status_claude`·`cli_login`
- `claude auth status` JSON 파싱 → `ClaudeAuthInfo { logged_in, auth_method, subscription_type, email }` 노출
- `Settings.auth_mode` (ApiKey/Cli) + `cli_versions: HashMap` 필드. `settings_write` 시 active_provider 또는 auth_mode 변경되면 build_provider rebuild
- 신규 에러 4종 — `NodeMissing`·`CliMissing`·`CliAuthRequired`·`CliRuntime`
- `CliSetupDialog.tsx` — 3단계(런타임 감지 → 설치 → 로그인) 진행 + 구독/콘솔 로그인 분기 + 에러 표시
- Settings → 프로바이더 탭 상단에 `auth.mode_card` 추가 (CLI 추천, API 키 백업)
- 단위 테스트 +8 (`cli_binary_path_unix`·`pkg_for_provider_matches_expected`·claude_cli JSONL 파서 5종 등)
- 디자인 — `design/v0.2.1_HANDOFF.md` 신규, `decision-log.md` D-066 추가
- 결정 (PR 24): #1 auth_mode 기본 ApiKey (v0.2 호환) — Cli 전환은 Settings/PR 27 Welcome에서 / #2 Anthropic만 우선 구현, Gemini/Codex는 PR 25/26 / #3 사용자 환경 격리 = `--tools "" --setting-sources "" --no-session-persistence` + cwd 강제 / #4 npm 전용 prefix `~/.airis/npm` (sudo 회피)

### Added (v0.2 PR 23) — v0.2 완성 🎉
- 자동 큐 워커 — `jobs::enqueue_or_update`에 exponential backoff next_retry_at 적용 (1m/2m/4m/8m, 4회 후 NULL → 수동만)
- `list_due_jobs` command — `next_retry_at <= NOW`인 잡 반환
- 프론트 자동 워커 — App.tsx 30초 polling, retryFailedJob 자동 호출. 결과는 chat:done 흐름으로 자연 통합
- F14.1 인앱 업데이트 알림 — `commands/updates.rs::check_for_update` (GitHub Releases API + SemVer 비교)
- 앱 시작 시 1회 + 24h throttle (localStorage `airis:update:last_check`)
- `UpdateDialog.tsx` — 새 버전 정보 + release notes preview + tauri-plugin-opener로 GitHub 페이지 open
- F14.2 SHA256 검증 표시 — release notes에 "sha256" 키워드 있으면 안내 표시 (release-pipeline.md 무서명 정책)
- 단위 테스트 +3 (semver newer·pre-release suffix·invalid)
- 결정 (PR 23): #1 자동 retry UX = chat:done 흐름 그대로 (별도 토스트 X — 자연 통합) / #2 업데이트 = 시작 시 + 24h throttle

### Added (v0.2 PR 22)
- F7.7 회상 챌린지 — `commands/recall.rs` (사용자가 챕터 핵심 적기 → paragraphs에서 빈도 top-8 키워드 추출 → 매치 비교)
- 통과 임계 60% (PASS_THRESHOLD) — 통과 시 자동 SRS 카드 생성 (F8.2 활성)
- DB 마이그 v7 — `recall_challenges` 테이블 (db-schema.md 그대로). expected/present/missing JSON 보관
- 키워드 추출 휴리스틱 — 영문/한글 ≥2자, 공백 분리 token, 빈도 정렬, 한·영 stop words 제외
- `RecallPanel.tsx` 슬라이드업 — 챕터 ref + textarea + 평가 결과 (expected/present/missing 색상별 badge)
- TopBar Lightbulb 아이콘 + `Mod+R` 단축키 (`uiStore.recallOpen`)
- 단위 테스트 +4 (top keywords 빈도·stop words 필터·한국어·normalize)
- 결정 (PR 22): F7.1 트리거 임계 = *모든 챕터 명시만*. 챕터 신뢰도 기반 자동 트리거(L1/L2)는 챕터 신뢰도 데이터 도입 후 v0.3+
- LLM 기반 평가는 v0.3+ (현재는 결정적 휴리스틱 — 비용 0)

### Added (v0.2 PR 21)
- F8 SRS — SuperMemo SM-2 알고리즘 (`commands/srs.rs::sm2_step` pure 함수, e_factor floor 1.3, 실패 시 reset)
- DB 마이그 v6 — `srs_cards` 테이블 (db-schema.md 그대로). FK study_slug, due_date 인덱스
- commands: `srs_add_card`·`srs_list_due`·`srs_review_card`·`srs_delete_card`
- 자동 due_date 계산 — std로 epoch → ISO 날짜 (chrono crate 의존 X). pomodoro의 `days_to_ymd_pub` 재사용
- `SrsPanel.tsx` 슬라이드업 — due 카드 차례차례, CSS transform rotateY로 flip 애니메이션 (framer-motion 도입 X)
- 평가 4단계 (again=0 / hard=3 / good=4 / easy=5) → SM-2 quality 매핑
- 카드 추가 다이얼로그 — front/back/section_ref 수동 입력
- TopBar Layers 아이콘 + `Mod+K` 단축키 (`uiStore.srsOpen`)
- 단위 테스트 +5 (sm2 first pass·second pass·실패 reset·기하 성장·e_factor floor)
- 결정 (PR 21): 카드 flip = CSS만 (A). framer-motion 도입은 v0.3+. 자동 카드 생성(F8.2)은 PR 22 회상 챌린지 통과 시 활성

### Added (v0.2 PR 20)
- F9 Pomodoro 타이머 — `commands/pomodoro.rs` (wall-clock 기반, started_at + duration_min만 저장 → OS sleep/wake에 정확)
- DB 마이그 v5 — `pomodoro_cycles` 테이블 (v2 누락분 보강). FK study_slug, phase CHECK, 인덱스
- AppState `pomodoro: Mutex<Option<PomodoroSession>>` — 단일 활성 세션
- start_pomodoro·stop_pomodoro·get_pomodoro_state commands
- 사이클 종료 시 pomodoro_cycles INSERT (completed/interruption 메타)
- `PomodoroPanel.tsx` 미니 패널 (우하단 floating) — 1초 polling, 25/5분 기본, 자동 만료 감지 + 자동 stop
- TopBar Timer 아이콘 + `Mod+Shift+P` 단축키 (`uiStore.pomodoroOpen`)
- 결정 (PR 20): wall-clock 기반 (B). OS 네이티브 알림은 v0.3+, 인앱 토스트만. 자동 세션 추적(F6.1)도 v0.3+
- 단위 테스트 +3 (db v5 1 + pomodoro persist 1 + format_iso 1 + leap year 1)

### Added (v0.2 PR 19) — v0.2c 시작
- F2.8/F12.2 stale 감지 — `commands/book::check_stale` (활성 스터디 모든 책의 source_path 현재 sha256 vs books.file_hash 비교, missing/changed 보고)
- `commands/book::reindex_book` — 변경된 파일의 hash·size 갱신 + start_indexing 흐름 호출
- `bookStore.staleByBookId`·`reindex`·`checkStale` — refresh 시 자동 stale 검사
- BookList 카드 stale 배지 (changed/missing) + 재인덱싱 버튼 (RotateCw 아이콘 + spinner)
- 단순화 결정 (PR 19): L3 폰트 클러스터링은 PR 19.5 (또는 v0.3)로 이연 — pdfium-render 폰트 API 검토 비용 큼. 회귀 테스트(F12.4/F12.5)도 v0.3+

### Added (v0.2 PR 18) — v0.2b 마무리
- DB 마이그 v4 — `intervention_signals`·`search_history`·`consistency_check_log` 테이블 추가 (db-schema.md 그대로)
- F7.2 반복 검색 감지 — `search_sections` 호출 시 search_history 적재 + query_norm 정규화(소문자·token sorted) + 30분 윈도우 N=3회 누적 시 `intervention_signals.repeat_search` 적재
- F12.1 Memory active 모순 검사 — `commands/consistency.rs` (Preferences/Corrections active 항목 키워드 겹침 휴리스틱). `memory_write` 후 자동 호출 → `consistency_check_log` 기록
- 정책: PR 18 시점엔 *데이터 누적*만, UI alert·signals 노출은 v0.3+
- 단위 테스트 +5 (db v4 1 + consistency 4)
- *결정 포인트 X* — 강도 명명은 PR 15에서 이미 confirm/auto/off로 박힘

### Added (v0.2 PR 17)
- F4.4 응답 검증 — `commands/validation.rs` (Memory.Corrections active 항목의 부정 패턴 추출 → 응답 매치 시 ViolationHit). 결정적 정규식만, LLM 검증은 v0.3+
- chat:done 직후 `emit_violations` hook — `chat:violation` event 발사
- chatStore `attachViolations` — 진행 중/직전 어시스턴트 메시지에 violations 첨부
- ChatMessage 노란 배너 — 위반 의심 항목 표시 (응답은 그대로, 거짓 양성 가능 명시)
- F4.5 3층 응답 — system prompt에 형식 안내 (요약 / 본문 인용 [1] / 더 알아보려면)
- ChatMessage `[1]`·`[2]` 인용 마커 인라인 강조 (badge 형태). 클릭 점프는 v0.3+
- 단위 테스트 +5 (validation: 위반 감지·미위반·resolved 무시·other section 무시·extract 안전성)
- 결정 (PR 17): 검증 위반 시 = 노란 배너 + 응답 그대로 (A). 강도 따른 재생성은 v0.3+

### Added (v0.2 PR 16)
- F10.5 `memory::compress` — 5섹션에서 *active 항목만* 추출 → L1(Preferences+Corrections, 2000자) + L2(Progress+Meta+Goals, 4000자)
- F10.6 `chat_send` 자동 주입 — Memory L1·L2를 system prompt 끝에 합성. 활성 섹션·검색 결과는 user message에
- D-036 prompt cache 활성 (Anthropic) — `ChatRequest.cache_breakpoints: Vec<CacheBreakpoint>` (System / Message(idx))
- AnthropicProvider build_request_body — cache_breakpoints 활용해 system block을 `[{type:text, text, cache_control:{type:ephemeral}}]` 형태로 wrap. 메시지 인덱스 cache_breakpoint도 동일 패턴
- OpenAI는 자동 prefix 캐싱(서버 측)이라 cache_breakpoints 무시. Gemini cachedContents는 v0.3+로 이연 (handoff 결정 #3)
- 단위 테스트 +5 (memory compress 3 + anthropic cache_control 2)
- 결정 (PR 16): #1 캐시 위치 메모리 / #2 cache_breakpoints 인덱스 (B) / #3 Gemini v0.3+

### Added (v0.2 PR 15)
- F10.3 발화 트리거 감지 — `commands/triggers.rs` 정규식 사전 (preference / correction / goal 분류, 한글·영문 패턴)
- `memory_detect_triggers`·`memory_apply_trigger` commands — 사용자 발화 → 트리거 hit → Memory 5섹션 자동 append
- `memory.rs::append_to_section` 헬퍼 — heading 발견 시 *그 섹션 안에* 항목 박음, 부재 시 새 섹션 생성
- `(active, since YYYY-MM-DD)` prefix 자동 — std로 epoch → ISO 날짜 (chrono crate 의존 X)
- F13.6 `Settings.intervention_level` (Confirm·Auto·Off) — 트리거 감지·갱신 정책
- `TriggerDialog.tsx` 1회 확인 다이얼로그 (우하단 floating, 매치 발화 + 추가될 항목 + 추가/건너뛰기)
- ChatPanel 통합 — 사용자 발화 직후 detect 호출 + 강도별 분기 (confirm 다이얼로그 / auto 즉시 적용 / off 비활성)
- Settings "강도" 탭 추가 — 3 옵션 라디오
- 의존성 추가: `regex` 1
- 단위 테스트 +9 (triggers 7 + memory append 2)
- 결정 (PR 15): 트리거 패턴 사전 = *코드 박음* (A). triggers.toml 외부 파일은 v0.3+ 검토

### Added (v0.2 PR 14)
- F10 Memory.md 표준 도입 — 사용자 성향·진도·이해도 누적 영역 (시스템 자동 갱신, 사용자 직접 편집 가능)
- `commands/memory.rs` — `MemoryDoc`(study·updated·body), 5섹션 헤딩 상수, frontmatter 파서/빌더 (Overview와 같은 정책)
- `memory_read`·`memory_write` commands — 원자적 쓰기(`.tmp` → atomic rename, SEQ-8) + mtime+sha256 fingerprint
- 외부 편집 감지 — 마지막 write fingerprint 모듈 단위 보관, read 시 비교 → `external_edited` 플래그
- 첫 read 시 default template 자동 반환 (5섹션 헤딩 포함)
- `components/MemoryEditor.tsx` 슬라이드업 패널 — 단일 textarea + 저장 + 외부 편집 경고 + 다시 불러오기
- TopBar에 Brain 아이콘 진입 + `Mod+M` 단축키 (`uiStore.memoryOpen` 글로벌 floating)
- 단위 테스트 +7 (parse round-trip, 폴백 슬러그, write/read round-trip, default template, 외부 편집 감지, fingerprint 매칭, 원자성 — tmp 잔류 X)
- 결정 (PR 14): 외부 편집 감지는 *로드 시점 mtime+hash 비교* (B). fs watcher는 v0.3+. Stronghold 폴백은 PR 14.5로 분리

### Added (v0.2 PR 13)
- 다중 LLM 프로바이더 — Anthropic + OpenAI + Gemini (D-005 부분 supersede)
- `settings::Provider` enum + `Settings.active_provider`·`models: HashMap<Provider, model>`
- 키 형식 검증 분기 — `sk-ant-` / `sk-` / `AIza` (각 prefix·최소 길이)
- `llm/openai.rs` — Chat Completions API + SSE + `[DONE]` 종료 + stream_options.include_usage
- `llm/gemini.rs` — `:streamGenerateContent?alt=sse` + `x-goog-api-key` + safety/blockReason 로그
- `AppState.llm: Mutex<Arc<dyn LlmProvider>>` — Settings.active_provider 변경 시 새 instance 교체. 진행 중 chat은 자기 Arc clone으로 끝까지 완료 (결정 #4)
- `lib::build_provider` 헬퍼 — Provider → Provider 인스턴스
- Settings UI 갱신 — "프로바이더" 탭 (활성 라디오 + 3개 카드 키 입력) / "모델" 탭 (활성 프로바이더 모델 셀렉터) / "언어" 탭
- ApiKeyInput placeholder 분기 — `PROVIDER_KEY_HINT`로 prefix·placeholder 표시
- ChatPanel 활성 프로바이더 키 검사 — `apiKeyPresent(active_provider)`
- 단위 테스트 +20 (openai 5 / gemini 7 / settings 4 / commands/settings 4)
- 결정 (handoff): #1 단일 active / #2 safety는 배너+응답 그대로(PR 17) / #3 정적 모델 목록 / #4 진행 중 챗 그대로 완료

### Added (v0.2 PR 12.6)
- 인앱 PDF 뷰어 — `pdfjs-dist` 5.7 + Tauri Asset Protocol 통합
- `tauri.conf.json` `app.security.assetProtocol` 활성 + scope (`$HOME/**`·`$APPDATA/**`·`$DOCUMENT/**`·`$DOWNLOAD/**`)
- Tauri `protocol-asset` feature 활성 (Cargo.toml)
- `BookContent.source_path` 추가 — PDF는 빈 content + source_path만, pdfjs가 `convertFileSrc`로 직접 로드
- BookViewer에 `PdfContent` 컴포넌트 — 페이지 캔버스 렌더 + 페이지 네비 (이전/다음 버튼·번호 입력)
- pdfjs worker 등록 — Vite `?url` 패턴
- `activeBookStore` `pendingPage` + `consumePendingPage` — 검색 결과 클릭 시 PDF 페이지 점프
- BookList — PDF 책 카드도 클릭 가능 (이전 disabled 가드 제거)
- 의존성 추가: `pdfjs-dist` 5.7
- 보안 표면 변경 명시 — assetProtocol scope 4개 home/data 디렉토리. v0.3에서 *동적 scope* (등록한 책만)으로 좁히기 검토

### Added (v0.2 PR 12.5)
- PDF 인덱싱 활성 — `start_indexing`이 PDF 분기 처리 (`parsers::pdf::parse` 호출)
- PDFium binary 동봉 — `scripts/setup-pdfium.sh` (Linux/macOS) + `pdfium-binaries` chromium/6996 다운로드 + `src-tauri/resources/pdfium/lib/`에 압축 해제
- `package.json` `pdfium:setup` 스크립트 + README 안내
- `tauri.conf.json` `bundle.resources` — pdfium lib·include·README placeholder 명시 (Tauri glob 매칭 안정)
- AppState `pdfium_lib_dir: Option<PathBuf>` 추가 — Tauri `resource_dir` 기반 자동 탐지. None이면 PDF 인덱싱 명시 안내 후 graceful skip
- `parsers::pdf::extract_from_text_fallback` 갱신 — 챕터 위치별 *페이지 본문 concat*. 챕터 없는 PDF는 단일 `Ch01`에 책 전체 본문 (검색 가능성 보존)
- AddBookDialog — PDF도 자동 인덱싱 호출. 안내 문구 정리 (시각 뷰어는 PR 12.6)
- `.gitignore` — `src-tauri/resources/pdfium/*` 추적 X, `PDFIUM_README.txt` placeholder만 유지

### Added (v0.2 PR 12)
- `BookViewer.tsx` — MD/HTML 책 뷰어. ReactMarkdown로 헤딩 렌더 시 클릭 가능. 활성 헤딩 시각 강조. 검색 결과·인용 클릭 시 ref 기반 anchor scroll
- HTML은 sandbox iframe + srcDoc (백엔드 ammonia sanitize와 이중 안전)
- TS heading slug 규칙 — Rust `parsers/slug.rs` 미러 (영/한 챕터 정규식·CJK 보존·dedupe)
- 백엔드 `book_read_raw` command — 책 raw content + format 반환 (PDF는 PR 12.5)
- 백엔드 `set_active_section`·`clear_active_section`·`get_active_section` commands + AppState `active_section` 캐시
- `chat_send` 컨텍스트 우선순위 변경 — *활성 섹션* (paragraphs WHERE book_id+section_path) → FTS5 검색 폴백 → current_file 폴백
- `BookList` 검색 입력 + 인라인 dropdown — 디바운스 300ms, 5 결과, FTS5 snippet `<<>>` → `<mark>` 변환
- `BookList` 책 카드 클릭 → `activeBookStore.open` → BookViewer 진입. 활성 책 시각 강조
- `Workspace` 라우팅 — 활성 책 있으면 BookViewer, 없으면 FileViewer (v0.1 fallback)
- 검색 결과 클릭 → `activeBookStore.jumpTo` (책 열기 + 섹션 점프 + 활성 박기) 일체 흐름
- `activeBookStore` (Zustand) — bookId·content·sectionPath·pendingScrollPath
- D-064 PR 12 정신 명시: 사용자 *명시 클릭*만 활성 — 자동 스크롤 추적 X (예측 가능성 우선)

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
