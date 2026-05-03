// 활성 스터디의 책 목록 캐시. 책 등록·삭제·인덱싱 후 refresh.

import { create } from "zustand";

import { api } from "@/lib/api";
import type { BookEntry, BookMetaInput } from "@/lib/types";

interface BookStore {
  books: BookEntry[];
  loaded: boolean;
  /** F2.8 stale 감지 결과 — book_id → status. UI 배지에 사용. */
  staleByBookId: Record<string, "changed" | "missing">;
  refresh: (studySlug: string) => Promise<void>;
  add: (
    studySlug: string,
    path: string,
    meta: BookMetaInput,
  ) => Promise<BookEntry>;
  remove: (studySlug: string, bookId: string) => Promise<void>;
  startIndexing: (studySlug: string, bookId: string) => Promise<void>;
  /** 변경된 책 재인덱싱 — hash·size 갱신 + paragraphs rebuild. */
  reindex: (studySlug: string, bookId: string) => Promise<void>;
  /** 활성 스터디의 모든 책 stale 감지. UI는 결과로 staleByBookId 갱신. */
  checkStale: (studySlug: string) => Promise<void>;
}

export const useBookStore = create<BookStore>((set, get) => ({
  books: [],
  loaded: false,
  staleByBookId: {},
  async refresh(studySlug) {
    if (!studySlug) {
      set({ books: [], loaded: true, staleByBookId: {} });
      return;
    }
    try {
      const books = await api.listBooks(studySlug);
      set({ books, loaded: true });
      // 부수적으로 stale 검사 — 결과는 별도 갱신.
      void get().checkStale(studySlug);
    } catch (e) {
      console.error("bookStore.refresh failed:", e);
      set({ books: [], loaded: true, staleByBookId: {} });
    }
  },
  async add(studySlug, path, meta) {
    const entry = await api.addMainBook(studySlug, path, meta);
    await get().refresh(studySlug);
    return entry;
  },
  async remove(studySlug, bookId) {
    await api.removeBook(studySlug, bookId);
    await get().refresh(studySlug);
  },
  async startIndexing(studySlug, bookId) {
    await api.startIndexing(studySlug, bookId);
    await get().refresh(studySlug);
  },
  async reindex(studySlug, bookId) {
    await api.reindexBook(studySlug, bookId);
    await get().refresh(studySlug);
  },
  async checkStale(studySlug) {
    try {
      const reports = await api.checkStale(studySlug);
      const map: Record<string, "changed" | "missing"> = {};
      for (const r of reports) {
        map[r.book_id] = r.status;
      }
      set({ staleByBookId: map });
    } catch (e) {
      console.error("bookStore.checkStale failed:", e);
    }
  },
}));
