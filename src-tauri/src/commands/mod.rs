// Tauri command 모듈 진입점.
// 각 sub 모듈 = 1개 도메인 (settings·llm·file 등 — features.md 매핑 표 참조).

pub mod file;
pub mod llm;
pub mod settings;
