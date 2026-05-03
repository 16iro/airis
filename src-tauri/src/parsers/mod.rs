// 책 파서 모듈 — F2 (책 등록·인덱싱)의 *결정적 코어* 부분.
//
// 4계층 섹션 모델 (design/repository-design.md 11.x):
//   L1 Book   — 책 자체 (uuid + 메타)
//   L2 Chapter— 인쇄 챕터 / 큰 단위 (예: "Ch04")
//   L3 Section— 챕터 내 절 (예: "§State")
//   L4 Paragraph — 검색 단위 (PR 11 임베딩에서 사용)
//
// 본 PR(10)은 L1·L2·L3까지(MD/HTML) + PDF 챕터 텍스트 폴백.
// L4 paragraph 분할 + 호출 commands는 PR 11에서 키워드/임베딩 인덱서·book commands가 처리.
//
// 섹션 ID 형식: `{book-uuid}/Ch04/§State` (의미 path)
//   * 책 UUID는 안정 — 책 자체는 거의 안 바뀜
//   * 섹션 path는 가독성 — 챗 인용 라벨과 동일 문자열로 표시 가능
//
// PR 10은 *라이브러리만*. commands::book(PR 11)에서 호출이 들어오면 dead_code 경고 자동 해소.
// 그때까지 모듈 단위 allow로 무경고 유지.
#![allow(dead_code)]

pub mod html;
pub mod markdown;
pub mod pdf;
pub mod slug;
pub mod types;
