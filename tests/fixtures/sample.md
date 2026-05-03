# 샘플 교재

이 파일은 airis v0.1 첫 슬라이스 동작 검증용 샘플입니다.

## 1. 결정적 코어 vs LLM 보강

airis는 **결정적 코어**(SRS·Pomodoro·트리거 패턴·Memory append)와 **LLM 보강**을 분리합니다.
LLM 실패 시 큐(`failed_llm_jobs`)로 적재되며 결정적 코어는 항상 작동합니다.

## 2. Local-First 약속

- 사용자 데이터·API 키는 *외부 서버 0건*
- 모든 인덱싱·캐싱은 사용자 머신에서 수행
- LLM 호출만 사용자 본인 키로 외부 발생

## 3. 기술 스택

- **Frontend**: Tauri 2 + React 19 + TypeScript + Vite
- **UI**: shadcn/ui + Tailwind v4 + Pretendard·Geist Mono
- **Backend**: Rust + Tokio + SQLite (WAL)

> 참고: 자세한 설계는 비공개 `design/` 디렉토리에서 관리됩니다.

```rust
// 결정적 코어 예시 — SRS SM-2 한 단계
fn sm2_step(card: &Card, rating: Rating) -> Card { ... }
```

## 다음 단계

이 파일을 챗 패널에 컨텍스트로 주입하여 학습 도우미와 대화해보세요.
