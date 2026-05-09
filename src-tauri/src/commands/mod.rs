// Tauri command 모듈 진입점.
// 각 sub 모듈 = 1개 도메인 (settings·llm·file 등 — features.md 매핑 표 참조).

pub mod ab_compare;
pub mod book;
pub mod cli_setup;
pub mod consistency;
pub mod dev_acceptance;
pub mod file;
pub mod hardware;
pub mod intervention;
pub mod learning_dev;
pub mod llm;
pub mod memory;
pub mod memory_facts;
pub mod overview;
pub mod pomodoro;
pub mod recall;
pub mod recall_v05;
pub mod search;
pub mod settings;
pub mod srs;
pub mod srs_generation;
pub mod study;
pub mod triggers;
pub mod updates;
pub mod validation;
