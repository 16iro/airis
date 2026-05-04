// 책 등록 폼 공유 타입·헬퍼 (PR 59).
// react-refresh/only-export-components 룰 때문에 BookFormCard.tsx와 분리.

export const SUPPORTED_BOOK_EXTS = [
  "md",
  "markdown",
  "html",
  "htm",
  "txt",
  "pdf",
];

export interface BookDraft {
  /** 클라이언트 측 임시 ID — list key. 백엔드는 다른 ID 부여. */
  id: string;
  path: string;
  title: string;
  author: string;
  /** 부교재 전용 — 챗 컨텍스트 헤더에 prepend되는 짧은 메모. */
  roleNote: string;
}

export function newBookDraftId(): string {
  return `draft-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export function inferTitleFromPath(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? "";
  return filename.replace(/\.[^.]+$/, "");
}
