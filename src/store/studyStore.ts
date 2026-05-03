// 활성 스터디 캐시 — 백엔드 studies.is_active의 미러.
// PR 8엔 active만 — list/CRUD UI는 PR 9 Library에서 추가.

import { create } from "zustand";

import { api } from "@/lib/api";
import type { StudyMeta } from "@/lib/types";

interface StudyStore {
  active: StudyMeta | null;
  loaded: boolean;
  /** 부팅 시 한 번 — 백엔드는 활성 스터디가 없으면 'default'를 자동 부트스트랩한다. */
  load: () => Promise<void>;
  /** 다른 스터디로 전환 — 백엔드 select_study 후 캐시 갱신. */
  select: (slug: string) => Promise<void>;
}

export const useStudyStore = create<StudyStore>((set) => ({
  active: null,
  loaded: false,
  async load() {
    try {
      const active = await api.getActiveStudy();
      set({ active, loaded: true });
    } catch (e) {
      console.error("studyStore.load failed:", e);
      // 실패해도 loaded=true로 — UI가 영원히 로딩 상태에 머물지 않도록.
      set({ active: null, loaded: true });
    }
  },
  async select(slug) {
    await api.selectStudy(slug);
    const active = await api.getActiveStudy();
    set({ active });
  },
}));
