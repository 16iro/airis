# airis

LLM 기반 교재 학습 보조 데스크톱 앱 — 사용자가 등록한 책(MD/PDF/HTML)을 검색·인용·회상·SRS·메타인지 제동까지 통합한 학습 도우미.
**Local-First** · 사용자 머신 외부 서버 0건 · 본인 LLM API 키 사용.

## 개발

```bash
# 의존성 설치
pnpm install

# 개발 (Vite + Tauri 한 번에)
pnpm tauri dev

# 빌드
pnpm tauri build

# 검사
pnpm typecheck
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## 스택

- **Frontend**: Tauri 2 + React 19 + TypeScript + Vite + Tailwind v4 + shadcn/ui
- **Backend (in-app)**: Rust + Tokio + SQLite (WAL)
- **LLM**: Anthropic SDK 기본 (사용자 API 키, 키체인 보관)

자세한 구조·결정 사항은 (비공개) `design/` 디렉토리.
