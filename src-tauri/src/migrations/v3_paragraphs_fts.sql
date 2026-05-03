-- v3 — 책 인덱싱: paragraphs (검색 단위) + FTS5 virtual table.
--
-- D-064 결정: v0.2엔 *FTS5 키워드 검색만*. 임베딩·하이브리드는 v0.3+로 이연.
--
-- 구조:
--   * paragraphs    = 검색의 *기본 단위*. 섹션 본문을 ~500자 청크로 분할 후 저장.
--   * paragraphs_fts = SQLite FTS5 virtual table. content=paragraphs로 외부 콘텐츠 모드 사용.
--                      한국어·영어를 unicode61 tokenizer로 처리 (음절 단위).
--   * triggers       = paragraphs INSERT/UPDATE/DELETE 시 FTS 인덱스 자동 동기화.
--
-- 섹션 경로 형식 (D-064): `{book_id-uuid}/Ch04/§State` — book_id는 따로, section_path는 책 내부.

-- 1) paragraphs 테이블 — 청크 단위 검색 본문 + 섹션·페이지 메타.
CREATE TABLE paragraphs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id       TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    section_path  TEXT NOT NULL,           -- "Ch04" 또는 "Ch04/§State"
    section_label TEXT NOT NULL,           -- 사람이 읽는 라벨 (예: "Ch04 §State")
    chunk_index   INTEGER NOT NULL,        -- 섹션 내 청크 순서 (0-base)
    content       TEXT NOT NULL,           -- 청크 본문
    page          INTEGER,                 -- PDF 페이지 (1-base, 옵션)
    char_offset   INTEGER NOT NULL DEFAULT 0,  -- 섹션 시작 기준 char offset
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_paragraphs_book_section ON paragraphs(book_id, section_path, chunk_index);

-- 2) FTS5 virtual table — content=paragraphs(content) 외부 콘텐츠 모드.
--    unicode61 tokenizer로 한국어 음절·영어 단어 모두 처리.
--    remove_diacritics 1 = 결합 문자(ǎ, é) 정규화.
CREATE VIRTUAL TABLE paragraphs_fts USING fts5(
    content,
    content='paragraphs',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 1'
);

-- 3) 동기화 트리거 — paragraphs CRUD를 FTS5에 자동 반영.
CREATE TRIGGER paragraphs_ai AFTER INSERT ON paragraphs BEGIN
    INSERT INTO paragraphs_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER paragraphs_ad AFTER DELETE ON paragraphs BEGIN
    INSERT INTO paragraphs_fts(paragraphs_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER paragraphs_au AFTER UPDATE ON paragraphs BEGIN
    INSERT INTO paragraphs_fts(paragraphs_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO paragraphs_fts(rowid, content) VALUES (new.id, new.content);
END;
