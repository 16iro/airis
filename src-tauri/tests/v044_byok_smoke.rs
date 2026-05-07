//! v0.4.4 PR 5 smoke test — D-095 BYOK 클라우드 임베딩 어댑터 + 라우팅 stub.
//!
//! 검증 (HANDOFF PR 5):
//!   1. `CloudEmbedder` trait 구현 — VoyageEmbedder가 dim·name을 정확히 노출.
//!   2. settings.byok_embedding round-trip — Some(ByokConfig)일 때 lowercase 직렬화.
//!   3. ByokProvider keyring_id가 LLM 키와 분리됨 (anthropic/openai/gemini 충돌 X).
//!   4. known_dim 매핑이 모델 ↔ vec0 차원 검증의 ground truth로 동작.
//!
//! 본 smoke 테스트는 *어댑터 trait + settings 통합*까지 검증. 실제 인덱싱 라우팅은
//! v0.4.4.1 또는 v0.4.5에서 박는다 — 본 PR 범위가 아님 (HANDOFF §1.5).

use airis_lib::index::v044::byok_embedding::{
    ByokConfig, ByokProvider, CloudEmbedder, VoyageEmbedder,
};

#[test]
fn voyage_embedder_implements_cloud_embedder_trait() {
    // dyn dispatch로 호출 가능한지 — 라우팅 시점에 어댑터 추상화가 동작하는지.
    let e: Box<dyn CloudEmbedder> =
        Box::new(VoyageEmbedder::new("test-key".into(), "voyage-3-lite".into()).unwrap());
    assert_eq!(e.name(), "voyage-3-lite");
    assert_eq!(e.dim(), 512);
}

#[test]
fn byok_config_round_trip_via_serde() {
    let cfg = ByokConfig {
        provider: ByokProvider::Voyage,
        model: "voyage-3-lite".into(),
    };
    let s = serde_json::to_string(&cfg).expect("serialize");
    let back: ByokConfig = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back, cfg);
    assert!(s.contains("\"provider\":\"voyage\""));
}

#[test]
fn byok_keyring_ids_distinct_from_llm_keys() {
    // settings/secrets에서 LLM 키와 BYOK 키가 *별도 entry*로 영속되는지 검증.
    // 이름 충돌 시 LLM provider 전환만으로 BYOK 키가 덮어쓰일 수 있어 보안 사고.
    let voyage_id = ByokProvider::Voyage.keyring_id();
    let gemini_byok_id = ByokProvider::Gemini.keyring_id();
    for llm in ["anthropic", "openai", "gemini"] {
        assert_ne!(
            voyage_id, llm,
            "Voyage BYOK 키 id가 LLM provider id와 충돌하면 안 됨"
        );
        assert_ne!(
            gemini_byok_id, llm,
            "Gemini BYOK 키 id가 LLM Gemini provider id와 충돌하면 안 됨"
        );
    }
    assert_ne!(voyage_id, gemini_byok_id);
}

#[test]
fn known_dim_covers_recommended_models() {
    // settings UI dropdown에 노출되는 모델은 모두 known_dim에 매핑되어 있어야
    // 사용자가 모델을 바꿔도 vec0 차원 검증이 작동.
    assert_eq!(ByokProvider::known_dim("voyage-3-lite"), Some(512));
    assert_eq!(ByokProvider::known_dim("voyage-3"), Some(1024));
    assert_eq!(ByokProvider::known_dim("text-embedding-004"), Some(768));
    // 알 수 없는 모델은 None — 어댑터가 응답 차원으로 보정.
    assert!(ByokProvider::known_dim("future-model").is_none());
}

#[test]
fn byok_provider_default_models_are_present_in_known_dim() {
    // ByokProvider::default_model()이 항상 known_dim에 있어야 dropdown 초기값으로 안전.
    let voyage_default = ByokProvider::Voyage.default_model();
    let gemini_default = ByokProvider::Gemini.default_model();
    assert!(ByokProvider::known_dim(voyage_default).is_some());
    assert!(ByokProvider::known_dim(gemini_default).is_some());
}
