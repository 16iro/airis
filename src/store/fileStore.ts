// 현재 열린 파일 상태 — 백엔드 AppState.current_file의 미러.
// 본문은 *백엔드만* 보관(LLM 호출 시 직접 주입)하지만, *FileViewer가 표시*하려면 프론트도 접근 필요.
// 처음 열 때 한 번만 본문을 받아오고, 이후엔 메모리에 캐시.

import { create } from "zustand";

import { api } from "@/lib/api";
import type { FileMeta } from "@/lib/types";

interface FileStore {
  meta: FileMeta | null;
  /** 본문 — FileViewer가 표시할 때만 사용. 챗 컨텍스트는 백엔드가 직접 주입. */
  content: string | null;
  loading: boolean;
  error: string | null;

  open: (path: string) => Promise<void>;
  close: () => Promise<void>;
  loadCurrent: () => Promise<void>;
}

export const useFileStore = create<FileStore>((set) => ({
  meta: null,
  content: null,
  loading: false,
  error: null,

  async open(path) {
    set({ loading: true, error: null });
    try {
      const meta = await api.fileOpen(path);
      const content = await api.fileCurrentContent();
      set({ meta, content, loading: false });
    } catch (e) {
      const message =
        typeof e === "object" && e !== null && "message" in e
          ? String((e as { message: string }).message)
          : String(e);
      set({ loading: false, error: message });
      throw e;
    }
  },

  async close() {
    await api.fileClose();
    set({ meta: null, content: null, error: null });
  },

  async loadCurrent() {
    const content = await api.fileCurrentContent();
    set({ content });
  },
}));
