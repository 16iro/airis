-- v15 — v0.4.2 PR 1 — 강건성 컬럼 + cascade 단계 칼럼 + cache 테이블.
--
-- D-073~D-080 위에 D-081~D-085 자리. v0.4.1 적재된 청크는 마이그 안에서 t1=done 백필.
--
-- 무파괴 원칙 (HANDOFF §5):
--   * 기존 chunks/vectors_t1/indexing_jobs는 *컬럼 추가만*. 데이터 변경은
--     v0.4.1 적재된 chunks의 embed_status_t1='done' 백필 1회뿐.
--   * forward-only — schema_version으로 누락분만 일괄 재실행. 재진입 시 ALTER가
--     중복 실패하지 않게 컬럼이 *없을 때만* 신설하고 싶지만, schema_version 가드가
--     이미 v15 재적용을 막기 때문에 단순 ALTER로 충분.
--
-- 신규 컬럼 (chunks):
--   * embed_status_t1 TEXT  — NULL / 'done' / 'failed'. tier-1(mE5-small)
--                              임베딩 단계의 현재 상태. v0.4.1 적재분은 백필.
--   * embed_status_t2 TEXT  — NULL / 'done' / 'failed'. tier-2(BGE-M3) 단계.
--                              v0.4.2 PR 2가 본격 채움.
--   * embed_attempts INTEGER NOT NULL DEFAULT 0
--                            — 같은 청크 임베딩 재시도 횟수. 3회 누적 후 skip
--                              (worker.rs가 책임). 무한 재시도 방지.
--   * last_error TEXT       — 최근 임베딩 실패 메시지 한 줄. 디버그/로그.
--
-- 신규 컬럼 (indexing_jobs):
--   * pause_reason TEXT     — NULL / 'user' / 'battery_low' / 'thermal' /
--                              'app_quit'. D-081 (PR 3) 우선순위 정책 위에서 PR 3가 채움.
--   * updated_at INTEGER    — epoch ms. 잡 상태 변경 시 worker.rs가 갱신.
--
-- 신규 테이블:
--   * vectors_t2          — BGE-M3(1024d) BLOB 페어. PR 2가 채움. vec0 가상
--                            테이블 `vectors_t2_vec0`은 v041 패턴대로 코드(PR 2의
--                            vector_store_t2)가 차원에 맞춰 생성한다.
--   * embedding_cache     — (D-084) 청크 텍스트 sha256 → 벡터 영속 cache.
--                            모델별 분리(text_hash + model이 사실상 합성키지만
--                            text_hash가 sha256(text + ':' + model)로 키 안에 모델 포함).
--                            여기선 단순화를 위해 text_hash PK + 인덱스로 모델 필터.
--   * response_cache      — (D-084) (notebook_id + rewritten_query +
--                            sorted(retrieved_chunk_ids) + active_model)의 sha256
--                            을 키로 LLM 응답 영속. PR 4가 본격 활용.

-- ----------------------------------------------------------------------------
-- 1) chunks 강건성 컬럼.
ALTER TABLE chunks ADD COLUMN embed_status_t1 TEXT;
ALTER TABLE chunks ADD COLUMN embed_status_t2 TEXT;
ALTER TABLE chunks ADD COLUMN embed_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN last_error TEXT;

-- v0.4.1에서 vectors_t1에 임베딩 적재된 청크는 이미 t1 단계 완료된 상태.
-- 마이그 안에서 1회 UPDATE. 이후엔 worker.rs가 책임.
UPDATE chunks
   SET embed_status_t1 = 'done'
 WHERE id IN (SELECT chunk_id FROM vectors_t1);

-- ----------------------------------------------------------------------------
-- 2) indexing_jobs 강건성 컬럼.
ALTER TABLE indexing_jobs ADD COLUMN pause_reason TEXT;
ALTER TABLE indexing_jobs ADD COLUMN updated_at INTEGER;

-- ----------------------------------------------------------------------------
-- 3) vectors_t2 — tier 2 임베딩 BLOB 영속 (BGE-M3, 1024d, little-endian f32).
--    vec0 가상 테이블 `vectors_t2_vec0`은 PR 2 vector_store_t2가 차원에 맞춰 생성.
CREATE TABLE vectors_t2 (
    chunk_id   INTEGER PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    embedding  BLOB    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);
-- chunk_id가 PRIMARY KEY라 별도 인덱스 불필요. KNN 시 vec0 가상 테이블이 책임.
-- (idx_vectors_t2_chunk는 chunks(id) → vectors_t2(chunk_id) JOIN 보조용)
CREATE INDEX idx_vectors_t2_chunk ON vectors_t2(chunk_id);

-- ----------------------------------------------------------------------------
-- 4) embedding_cache — 청크 텍스트 sha256 → 벡터 BLOB 영속 (D-084).
--    text_hash는 PR 4 cache::embedding이 sha256(text)로 계산. model로 차원 분리.
--    LRU eviction은 PR 4가 last_hit_at ASC LIMIT N으로 책임.
CREATE TABLE embedding_cache (
    text_hash    TEXT    PRIMARY KEY,
    embedding    BLOB    NOT NULL,
    model        TEXT    NOT NULL,
    dim          INTEGER NOT NULL,
    created_at   INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    last_hit_at  INTEGER
);
CREATE INDEX idx_embedding_cache_model    ON embedding_cache(model);
CREATE INDEX idx_embedding_cache_last_hit ON embedding_cache(last_hit_at);

-- ----------------------------------------------------------------------------
-- 5) response_cache — LLM 응답 영속 cache (D-084).
--    key = sha256(notebook_id + rewritten_query + sorted(retrieved_chunk_ids) + active_model).
--    notebook_id 컬럼은 v0.4.1 컨벤션 그대로 'book_id'(=books.id, TEXT/UUID)를 담는다.
CREATE TABLE response_cache (
    key            TEXT    PRIMARY KEY,
    notebook_id    TEXT    NOT NULL,
    response_text  TEXT    NOT NULL,
    model          TEXT    NOT NULL,
    created_at     INTEGER NOT NULL,
    last_hit_at    INTEGER,
    hit_count      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_response_cache_book     ON response_cache(notebook_id);
CREATE INDEX idx_response_cache_last_hit ON response_cache(last_hit_at);
