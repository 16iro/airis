<div align="center">

<img src="public/logo-readme.svg" alt="airis logo" width="128" height="128" />

# airis

**LLM 기반 교재 학습 보조 데스크톱 앱**

책을 *읽다가* AI에게 그 자리에서 묻고, 메타인지 제동·SRS·회상까지 한 곳에서.

[![Tauri](https://img.shields.io/badge/Tauri-2-C8FF3D?style=flat-square&labelColor=1A1A1A)](https://tauri.app)
[![React](https://img.shields.io/badge/React-19-C8FF3D?style=flat-square&labelColor=1A1A1A)](https://react.dev)
[![Rust](https://img.shields.io/badge/Rust-stable-C8FF3D?style=flat-square&labelColor=1A1A1A)](https://www.rust-lang.org)
[![status](https://img.shields.io/badge/status-active%20development-C8FF3D?style=flat-square&labelColor=1A1A1A)](#로드맵)
[![local-first](https://img.shields.io/badge/local--first-yes-C8FF3D?style=flat-square&labelColor=1A1A1A)](#설계-원칙)

</div>

---

## 왜 airis인가

기존 RAG 챗봇이 못 채우는 *학습 동반자*의 자리.

- **Local-First** — 사용자 머신 밖으로 데이터 안 나감. LLM 호출만 외부, 그것도 본인 구독·키 사용
- **이중과금 회피** — Claude Pro / ChatGPT Plus / Gemini Advanced 구독자가 *추가 API 과금 없이* CLI subprocess로 호출. API 키는 advanced 폴백
- **메타인지 제동** — 페이스 vs 마감 비교, 목표 챕터 정렬, 학습 속도 자기 인식
- **인용 가능 컨텍스트** — 책 본문 섹션이 답변 근거로 박힘. 환각 최소화 + 출처 추적 가능
- **장기 학습 도구** — 챗 기록 / SRS 카드 / 회상 챌린지 / Memory.md 누적이 노트북 단위로 격리·보존

## 주요 기능

| 영역 | 내용 |
|---|---|
| **책 등록** | MD / HTML / PDF / TXT 자동 파싱·인덱싱. 주교재 1권 + 부교재 N권 모델 |
| **검색·챗** | 책 본문 컨텍스트 자동 주입, `[Sx]` 인용 마커 + 컨텍스트 칩으로 출처 시각화 |
| **워크스페이스** | dockview 기반 자유 패널 배치, 9개 패널 (TOC·뷰어·챗·퀴즈·노트·SRS·진도·기록·뽀모도로) |
| **학습 보조** | SRS 카드 / 회상 챌린지 / Pomodoro / 학습 기록 자동 누적 |
| **스터디 라이프사이클** | 마법사 생성 → 인덱싱 진행 → 챗 → 표지·목표·마감일 편집 → 데이터 폴더 열기 |
| **프로바이더** | Anthropic Claude / OpenAI / Google Gemini — CLI subprocess 우선, API 키 폴백 |
| **i18n / 테마** | 한국어 단일 (현재) · 라이트·다크 자동 / 수동 · 강조 색 프리셋 (sky / orange / lime) |

## 설계 원칙

1. **사용자 머신이 곧 서버다** — SQLite WAL, 로컬 파일시스템, 외부 서비스 의존성 0
2. **구독 우선, API 폴백** — 이미 가진 LLM 구독을 활용. 이중과금 X
3. **모든 결정은 사용자가 봐야 한다** — 챗 응답에 컨텍스트가 박혀 있고, 인용 마커는 클릭 가능 (v0.4.1+)
4. **학습 흐름이 휘발되지 않는다** — 모든 사용자 자산(스터디 / 책 / 챗 / Memory / SRS)은 사람이 읽을 수 있는 파일·SQLite로 영속

## 빠른 시작

```bash
# 1. 의존성 설치
pnpm install

# 2. PDFium 바이너리 다운로드 (PDF 인덱싱용, ~5MB)
pnpm pdfium:setup

# 3. 개발 서버 시작
pnpm tauri dev
```

처음 실행 시 사용 중인 LLM CLI(`claude` / `gemini` / `codex`)가 설치되어 있으면 자동 인증 안내, 아니면 Settings에서 API 키 입력.

## 빌드

```bash
# 릴리스 빌드
pnpm tauri build

# 검증 — 모든 PR이 통과해야 머지
pnpm typecheck
pnpm lint
pnpm test:unit
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

릴리스 binary 약 30MB (v0.3.2 기준, Linux x86_64).

## 스택

| 레이어 | 기술 |
|---|---|
| **Shell** | Tauri 2 |
| **UI** | React 19 + TypeScript + Tailwind v4 + shadcn/ui + dockview |
| **Backend** | Rust + Tokio + SQLite (WAL) + rusqlite |
| **Search** | SQLite FTS5 (v0.4부터 sqlite-vec 하이브리드) |
| **LLM** | Anthropic / OpenAI / Gemini (CLI subprocess 우선, API 키 폴백) |
| **PDF** | pdfium-render |
| **Auth** | OS keychain (`keyring` crate) |
| **i18n** | i18next (현재 ko 단일) |
| **Toast** | sonner |

## 프로젝트 구조

```
airis/
├── src/                  # React 프론트엔드
│   ├── components/       # 패널·다이얼로그·UI 요소
│   ├── pages/            # Library / Workspace / Welcome
│   ├── store/            # zustand 스토어
│   ├── lib/              # api·types·toast·utils
│   └── locales/          # ko.json (i18n)
├── src-tauri/            # Rust 백엔드
│   ├── src/commands/     # Tauri command 모듈 (study/book/llm/srs/...)
│   ├── src/parsers/      # MD/HTML/PDF 파서
│   ├── src/index/        # 청커·키워드 인덱서
│   ├── src/llm/          # 프로바이더 어댑터
│   └── src/migrations/   # SQLite 스키마 v1~v12
└── public/
    └── logo.svg
```

## 로드맵

| 시리즈 | 상태 | 핵심 |
|---|---|---|
| **v0.1** | 완료 | 기본 셸 + 첫 LLM 호출 + 책 1권 모델 |
| **v0.2.x** | 완료 | 다중 스터디 / FTS 검색 / Memory / SRS / 메타인지 / CLI 브릿지 |
| **v0.3.x** | 완료 | UI/UX 정립 (prototype 충실) / dockable workspace / 스터디 라이프사이클 / 토스트 / 챗 컨텍스트 시각화 |
| **v0.4.x** | 진행 예정 | RAG 엔진 정립 — fastembed-rs + sqlite-vec 하이브리드, 컨텍스트 파이프라인 (HyDE·sentence window·인용 검증), 비-MD 포맷 1급 시민화 |
| **v0.5+** | 계획 | 학습 레이어(Memory/SRS/메타인지) 새 RAG 위에서 재정합 |

상세 설계·결정 사항은 비공개 `design/` 디렉토리.

## 검증된 사용 흐름 (v0.3.2 기준)

- 스터디 생성 → 주/부교재 등록 → 백그라운드 인덱싱 → 챗
- 라이브러리 카드 검색 (⌘K) / 인스펙터 / 더블클릭 즉시 진입
- 워크스페이스 9개 패널 자유 배치 + 레이아웃 persist + 리셋 버튼
- 표지·이름·설명·학습 목표·마감일 편집 + 데이터 폴더 OS 매니저로 열기
- 챗 응답에 컨텍스트 칩(어느 섹션·책이 인용됐는지) 표시
- TOC 안 검색 / 인스펙터에서 마지막 챗 미리보기
- 토스트 시스템 (저장 성공·삭제·인덱싱 등) / 강조 색 프리셋 / 라이트·다크 토글

## 상태 / 라이센스

활발한 개발 중. 별도 라이센스 미설정 — 개인 학습 프로젝트.
