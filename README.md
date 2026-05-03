# airis

LLM 기반 교재 학습 보조 데스크톱 앱 — 사용자가 등록한 책(MD/PDF/HTML)을 검색·인용·회상·SRS·메타인지 제동까지 통합한 학습 도우미.
**Local-First** · 사용자 머신 외부 서버 0건 · 본인 LLM API 키 사용.

## 개발

```bash
# 의존성 설치
pnpm install

# PDFium binary 다운로드 (PDF 인덱싱·뷰어 — ~5MB libpdfium.so)
pnpm pdfium:setup

# 개발 (Vite + Tauri 한 번에)
pnpm tauri dev

# 빌드
pnpm tauri build

# 검사
pnpm typecheck
pnpm lint
pnpm test:unit
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --lib
```

> `pnpm pdfium:setup`은 첫 빌드 전에 *한 번* 실행. PDFium binary는 git 추적 X — 빌드 시 Tauri resources에 동봉되어 사용자 빌드된 앱에 자동 포함. 미설치 시 cargo build 실패 (Tauri resources glob).

## 스택

- **Frontend**: Tauri 2 + React 19 + TypeScript + Vite + Tailwind v4 + shadcn/ui
- **Backend (in-app)**: Rust + Tokio + SQLite (WAL)
- **LLM**: Anthropic SDK 기본 (사용자 API 키, 키체인 보관)

자세한 구조·결정 사항은 (비공개) `design/` 디렉토리.
