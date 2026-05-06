<div align="center">

<img src="public/logo-readme.svg" alt="airis logo" width="128" height="128" />

</div>

# airis

`airis`는 데스크톱 환경에서 동작하는 **로컬 RAG 엔진 + 학습 워크스페이스**입니다. Tauri 2 셸 위에서 React 프론트엔드와 Rust 백엔드가 결합되어 있고, 책(PDF/Markdown/HTML/TXT) 한 권을 청킹·임베딩한 뒤 hybrid 검색(`sqlite-vec` KNN + SQLite FTS5 + RRF)으로 컨텍스트를 구성하여 LLM CLI(`claude-code` / `gemini-cli` / `codex-cli`)에 subprocess 브릿지로 전달합니다. 임베딩 모델(`multilingual-e5-small` / `BGE-M3`)은 ONNX로 디바이스 안에서만 추론하며, 사용자 책 데이터·OAuth 토큰은 외부 서버로 나가지 않습니다.

본 저장소는 **단일 작성자의 개인 프로젝트**이며 현재 `v0.4.x` 시리즈에서 RAG 엔진을 본격 정립 중입니다 (`v0.4.2` 기준).

---

## Build

### 사전 요구

| 도구 | 최소 버전 | 비고 |
|---|---|---|
| Node.js | 20 | |
| pnpm | 10 | `corepack enable` 권장 |
| Rust | stable | `rustup install stable` |
| Tauri 2 OS 빌드 도구 | — | [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/) |
| LLM CLI 또는 API 키 | — | `claude` / `gemini` / `codex` 중 하나가 PATH에 있거나 Anthropic/OpenAI/Google API 키 |

### 개발 빌드

```bash
pnpm install
pnpm pdfium:setup    # PDFium 7825+ 다운로드 (PDF 파싱용, ~7MB)
pnpm tauri dev
```

`pnpm pdfium:setup`은 [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)에서 OS·아키텍처별 prebuilt를 받아 `src-tauri/resources/pdfium/`에 배치합니다. 환경변수 `PDFIUM_VERSION=<revision>`으로 override 가능합니다.

### 릴리스 빌드

```bash
pnpm tauri build
```

산출물은 `src-tauri/target/release/bundle/`에 생성됩니다. `v0.4.x` 기준 binary 약 62 MB이며, 임베딩 모델(T1 mE5-small ~120 MB / T2 BGE-M3 ~2 GB)은 첫 인덱싱 시 사용자 머신에 다운로드됩니다 (binary에 포함되지 않음).

### 검증

```bash
# 백엔드
cargo clippy --release --all-targets -- -D warnings
cargo test --release

# 프론트엔드
pnpm typecheck
pnpm lint
pnpm test:unit
```

PR 머지 전 위 다섯 가지 모두 통과가 필수입니다.

---

## How it works

### 인덱싱 파이프라인

```
책 파일
  └─→ parsers (md / html / pdf::parse via pdfium-render / txt)
       └─→ index/v041/chunker
            ├─ MD/HTML: 섹션 헤더가 부모, 본문을 800~1200 토큰 윈도우로 재귀 분할
            ├─ PDF:     페이지가 부모, 페이지 본문을 동일 윈도우로 분할
            └─ 문장 경계 보존: icu_segmenter 2 (한국어 종결어미 인식)
       └─→ index/v042/worker::embed_batch
            ├─ T1: fastembed multilingual-e5-small (INT8, 384d)
            ├─ T2: fastembed BGE-M3 (FP, 1024d, 백그라운드)
            └─ 단일 트랜잭션: vec0 + chunks.embed_status + indexing_jobs.progress 동시 commit
       └─→ chunks 테이블 + chunks_fts (FTS5 트리거 자동 동기화) + vectors_t{1,2}
```

### 검색·응답 파이프라인

```
사용자 질의
  └─→ commands::llm::build_v041_block
       ├─ active_index.txt 읽기 → T1 (default) 또는 T2
       ├─ embedder.embed_query → vector top-K (sqlite-vec KNN)
       ├─ chunks_fts MATCH      → FTS5 top-K
       └─ RRF 병합 (k=60) → top-N retrieved chunks
  └─→ index/v041/context::build_context
       ├─ 시스템 프롬프트 (한국어 few-shot 2건, 인용 [Sx] 강제)
       ├─ 메타데이터 블록 [Sx] (책·페이지·section_path)
       └─ 토큰 예산 패킹 (점수 오름차순 reverse — Lost in the Middle 회피)
  └─→ llm::ProviderAdapter (claude-code / gemini-cli / codex-cli / API 키)
  └─→ 응답 SSE 스트림 + parse_citations → ChatMessage UI에 [Sx] 클릭 가능 chip
```

### 강건성·자원 제어

- **WAL + 트랜잭션 체크포인트**: SIGKILL 시 손실 ≤ 1 배치 (`worker.rs::embed_batch`).
- **재개 메커니즘**: 앱 재시작 시 `resume_pending_jobs`가 `chunks.embed_status_t{1,2}` 기준으로 미완료 청크만 재처리.
- **일시정지 4 트리거**: 우선순위 `user > app_quit > thermal > battery_low > cooperative_chat`. UPower D-Bus(Linux)·`SystemEvents.PowerModeChanged`(Windows, stub)·`IOPSNotification`(macOS, stub).
- **자원 제한**: T2 빌드 중 `setpriority(nice 10)` / `IDLE_PRIORITY_CLASS` + `OMP_NUM_THREADS` 절반. 사용자 chat 진입 시 cooperative pause.
- **캐시**: `embedding_cache` (sha256(text+model) → 벡터) + `response_cache` (sha256(book+query+chunks+model) → 응답, 7일 TTL). SQLite + 인메모리 LRU 1024.

자세한 결정 근거는 작업 메모(`design/decision-log.md`, 비공개)의 `D-073` ~ `D-085` 항목입니다.

---

## Repository layout

```
airis/
├── src/                         # React 프론트엔드 (TypeScript strict)
│   ├── components/              # 패널·다이얼로그·UI 요소
│   │   ├── AbComparePanel.tsx   # baseline vs v041_hybrid A/B 비교 dev 모드
│   │   ├── BookFormCard.tsx     # 책 카드 + 재인덱싱 버튼
│   │   └── ChatMessage.tsx      # [Sx] 인용 chip 클릭 점프
│   ├── pages/                   # Library / Workspace / Welcome / Settings
│   ├── store/                   # Zustand 슬라이스 (study/chat/memory/pomodoro/...)
│   └── locales/ko.json          # i18n (한국어 단일)
│
├── src-tauri/                   # Rust 백엔드 (Tokio · rusqlite)
│   ├── src/
│   │   ├── commands/            # Tauri command 모듈
│   │   ├── parsers/             # md / html / pdf / txt 파서
│   │   ├── index/
│   │   │   ├── v041/            # chunker · embedder T1 · retrieval · context
│   │   │   └── v042/            # worker · resume · cascade · manifest · throttle
│   │   ├── cache/               # embedding_cache · response_cache
│   │   ├── llm/                 # 프로바이더 어댑터
│   │   ├── power_monitor/       # OS-별 일시정지 트리거
│   │   ├── runtime/             # 자원 제한 (nice / priority class)
│   │   └── migrations/          # SQLite 스키마 v1 ~ v16
│   └── resources/pdfium/        # pdfium-binaries 동봉 (gitignored)
│
├── scripts/                     # setup-pdfium.sh
└── public/                      # 로고·정적 자산
```

---

## Stack

| 레이어 | 기술 |
|---|---|
| Shell | Tauri 2 (Rust + WebView) |
| UI | React 19 · TypeScript strict · Tailwind v4 · shadcn/ui · dockview |
| State | Zustand |
| Backend | Rust · Tokio · rusqlite (SQLite WAL) |
| 임베딩 | [fastembed-rs](https://github.com/Anush008/fastembed-rs) 5.x — `multilingual-e5-small` INT8 (384d) / `BGE-M3` FP (1024d) |
| 벡터 검색 | [sqlite-vec](https://github.com/asg017/sqlite-vec) 0.1.9 (Rust crate, C 소스 static link) |
| 키워드 검색 | SQLite FTS5 |
| 청킹 | [text-splitter](https://crates.io/crates/text-splitter) 0.30 + [icu_segmenter](https://crates.io/crates/icu_segmenter) 2 |
| LLM 어댑터 | claude-code / gemini-cli / codex-cli subprocess + Anthropic / OpenAI / Gemini API 키 폴백 |
| PDF | [pdfium-render](https://crates.io/crates/pdfium-render) 0.8 + [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) 7825+ |
| 키 관리 | OS 키체인 ([keyring](https://crates.io/crates/keyring) crate) |

OS-별 의존성은 `cfg`-gated으로 분리되어 있습니다 (예: `zbus = "5"`는 Linux target에서만 컴파일).

---

## Status

| Phase | 버전 | 상태 |
|---|---|---|
| 0 PoC | v0.4.0 | 완료 (gate 5/5 PASS) |
| 1 단일 노트북 MVP | v0.4.1 | 완료 (DB v13 · chunks · hybrid 검색 · [Sx] 점프) |
| 2 cascade · 강건성 · 캐시 | v0.4.2 | 완료 (DB v15~v16 · T2 BGE-M3 · 일시정지 4트리거 · cache) |
| 3 검색·응답 품질 | v0.4.3 | 미시작 (Query rewriting · HyDE · Reranker · 대화 압축) |
| 4 다양화 | v0.4.4 | 미시작 (DOCX · BYOK · gemini/codex 안정화) |

### 동작 확인된 흐름

- MD / HTML / TXT / text-layer PDF 책 등록·인덱싱·검색
- Hybrid 검색 (sqlite-vec + FTS5 + RRF)
- `[Sx]` 인용 chip 클릭 → BookViewer 페이지·섹션 점프
- 일시정지/재개 (사용자·배터리·절전·SIGKILL)
- claude-code subprocess 브릿지 (Claude Max OAuth로 100건 호출, 추가 청구 0원 검증)

### 미동작·후속 슬라이스 예정

- DOCX / 스캔 PDF (OCR) / YouTube / 오디오 — `v0.4.4`
- Reranker (cross-encoder 인용 검증) — `v0.4.3`
- Query rewriting / HyDE — `v0.4.3`

---

## Known issues

| ID | 영향 | 우회 | 대응 |
|---|---|---|---|
| BUG-001 | `gemini-cli` stream 응답이 cumulative full text를 delta로 잘못 처리 → 누적 prefix 형태로 표시 | provider를 Claude로 전환 | `v0.4.4` |
| BUG-002 | 일부 provider에서 응답 전체가 통째로 3회 반복 (재현 조건·원인 미상) | provider 변경, 재현 시 dev console에서 `chat:*` 이벤트 횟수 확인 | `v0.4.4` |

---

## Credits

핵심 의존성 작성자:

- 임베딩: [Anush008/fastembed-rs](https://github.com/Anush008/fastembed-rs) · [intfloat/multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small) (Microsoft Research) · [BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3)
- 벡터: [asg017/sqlite-vec](https://github.com/asg017/sqlite-vec) (Alex Garcia)
- 청킹: [benbrandt/text-splitter](https://github.com/benbrandt/text-splitter) · [unicode-org/icu4x](https://github.com/unicode-org/icu4x)
- PDF: [ajrcarey/pdfium-render](https://github.com/ajrcarey/pdfium-render) · [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)
- Shell·UI: [Tauri](https://tauri.app) · [shadcn/ui](https://ui.shadcn.com) · [dockview](https://dockview.dev)
- LLM CLI: [Claude Code](https://docs.claude.com/en/docs/claude-code) · [gemini-cli](https://github.com/google-gemini/gemini-cli) · [codex-cli](https://github.com/openai/codex)

---

## License

별도 라이센스 미명시. 본 저장소를 *그대로 사용*하거나 *포크*하려는 의향이 있으면 [Issues](https://github.com/16iro/airis/issues)로 문의 바랍니다.
