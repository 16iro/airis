// 활성 스터디 캐시 — 백엔드 studies.is_active의 미러.
// PR 8엔 active만 — list/CRUD UI는 PR 9 Library에서 추가.

import { create } from "zustand";

import { api } from "@/lib/api";
import type { StudyMeta } from "@/lib/types";

interface StudyStore {
  active: StudyMeta | null;
  list: StudyMeta[];
  loaded: boolean;
  /** 부팅 시 한 번 — 백엔드는 활성 스터디가 없으면 'default'를 자동 부트스트랩한다. */
  load: () => Promise<void>;
  /** Library 진입 시 호출 — 전체 목록 갱신. */
  refreshList: () => Promise<void>;
  /** 다른 스터디로 전환 — 백엔드 select_study 후 캐시 갱신. */
  select: (slug: string) => Promise<void>;
  /** 새 스터디 생성 + 캐시 갱신 + (자동 활성된 경우) 활성으로 박힘.
   *  슬러그는 백엔드가 이름에서 자동 도출(한국어 그대로 + 충돌 시 ` (2)` suffix). */
  create: (name: string, language?: string) => Promise<StudyMeta>;
  /** 스터디 영구 삭제 — 백엔드는 삭제 후 다른 활성 스터디로 자동 전환. */
  remove: (slug: string) => Promise<void>;
}

export const useStudyStore = create<StudyStore>((set, get) => ({
  active: null,
  list: [],
  loaded: false,
  async load() {
    try {
      const [active, list] = await Promise.all([
        api.getActiveStudy(),
        api.listStudies(),
      ]);
      set({ active, list, loaded: true });
    } catch (e) {
      console.error("studyStore.load failed:", e);
      set({ active: null, list: [], loaded: true });
    }
  },
  async refreshList() {
    const list = await api.listStudies();
    set({ list });
  },
  async select(slug) {
    await api.selectStudy(slug);
    const active = await api.getActiveStudy();
    set({ active });
    await get().refreshList();
  },
  async create(name, language) {
    const study = await api.createStudy(name, language ?? null);
    await get().refreshList();
    if (study.is_active) {
      set({ active: study });
    }
    return study;
  },
  async remove(slug) {
    await api.deleteStudy(slug, true);
    const active = await api.getActiveStudy();
    set({ active });
    await get().refreshList();
  },
}));
