// Tauri command 모듈 진입점.
// 각 sub 모듈 = 1개 도메인 (settings·llm·search 등 — features.md 매핑 표 참조).
// v0.1 PR 3 시점엔 settings만 활성.

pub mod settings;
