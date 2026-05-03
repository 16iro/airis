// 활성 책 + 활성 섹션 캐시.
//
// 흐름:
//   * BookList에서 책 클릭 → activeBookStore.open(study, bookId) → BookViewer 표시
//   * BookViewer에서 헤딩 클릭 → activeBookStore.setSection(path) → 백엔드 set_active_section
//   * chat_send는 백엔드 active_section을 컨텍스트 우선순위 1로 사용

import { create } from "zustand";

import { api } from "@/lib/api";
import type { BookContent } from "@/lib/types";

interface ActiveBookStore {
  bookId: string | null;
  content: BookContent | null;
  loading: boolean;
  /** 클릭된 헤딩의 section_path (앵커 점프 + 백엔드 컨텍스트). */
  sectionPath: string | null;
  /** 검색 결과·인용 클릭 시 BookViewer가 anchor scroll할 대상. 사용 후 null. */
  pendingScrollPath: string | null;

  open: (studySlug: string, bookId: string) => Promise<void>;
  close: () => Promise<void>;
  setSection: (sectionPath: string) => Promise<void>;
  /** 검색 결과 클릭 시 — 책 열기 + 점프 대상 박기. */
  jumpTo: (studySlug: string, bookId: string, sectionPath: string) => Promise<void>;
  consumePendingScroll: () => string | null;
}

export const useActiveBookStore = create<ActiveBookStore>((set, get) => ({
  bookId: null,
  content: null,
  loading: false,
  sectionPath: null,
  pendingScrollPath: null,

  async open(studySlug, bookId) {
    if (get().bookId === bookId && get().content) {
      return;
    }
    set({ loading: true });
    try {
      const content = await api.bookReadRaw(studySlug, bookId);
      set({
        bookId,
        content,
        loading: false,
        sectionPath: null,
        pendingScrollPath: null,
      });
    } catch (e) {
      console.error("activeBookStore.open failed:", e);
      set({ loading: false, content: null, bookId: null });
      throw e;
    }
  },

  async close() {
    await api.clearActiveSection().catch(() => {});
    set({
      bookId: null,
      content: null,
      sectionPath: null,
      pendingScrollPath: null,
    });
  },

  async setSection(sectionPath) {
    const { bookId } = get();
    if (!bookId) return;
    set({ sectionPath });
    try {
      await api.setActiveSection(bookId, sectionPath);
    } catch (e) {
      console.error("setActiveSection failed:", e);
    }
  },

  async jumpTo(studySlug, bookId, sectionPath) {
    await get().open(studySlug, bookId);
    set({ pendingScrollPath: sectionPath });
    await get().setSection(sectionPath);
  },

  consumePendingScroll() {
    const path = get().pendingScrollPath;
    if (path) set({ pendingScrollPath: null });
    return path;
  },
}));
