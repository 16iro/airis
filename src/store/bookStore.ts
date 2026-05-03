// 활성 스터디의 책 목록 캐시. 책 등록·삭제·인덱싱 후 refresh.

import { create } from "zustand";

import { api } from "@/lib/api";
import type { BookEntry, BookMetaInput } from "@/lib/types";

interface BookStore {
  books: BookEntry[];
  loaded: boolean;
  refresh: (studySlug: string) => Promise<void>;
  add: (
    studySlug: string,
    path: string,
    meta: BookMetaInput,
  ) => Promise<BookEntry>;
  remove: (studySlug: string, bookId: string) => Promise<void>;
  startIndexing: (studySlug: string, bookId: string) => Promise<void>;
}

export const useBookStore = create<BookStore>((set, get) => ({
  books: [],
  loaded: false,
  async refresh(studySlug) {
    if (!studySlug) {
      set({ books: [], loaded: true });
      return;
    }
    try {
      const books = await api.listBooks(studySlug);
      set({ books, loaded: true });
    } catch (e) {
      console.error("bookStore.refresh failed:", e);
      set({ books: [], loaded: true });
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
}));
