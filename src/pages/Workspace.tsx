// 워크스페이스 — TopBar + BookList + (BookViewer 또는 FileViewer + ChatPanel) 2-pane.
// PR 12: 활성 책 있으면 BookViewer, 없으면 FileViewer (v0.1 호환 fallback).

import { BookList } from "@/components/BookList";
import { BookViewer } from "@/components/BookViewer";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { TopBar } from "@/components/TopBar";
import { useActiveBookStore } from "@/store/activeBookStore";

interface Props {
  registerChatHandle?: Parameters<typeof ChatPanel>[0]["registerHandle"];
}

export function Workspace({ registerChatHandle }: Props) {
  const activeBookId = useActiveBookStore((s) => s.bookId);
  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <TopBar />
      <BookList />
      <div className="grid flex-1 grid-cols-1 overflow-hidden md:grid-cols-[1fr_400px]">
        <div className="overflow-hidden border-r border-border">
          {activeBookId ? <BookViewer /> : <FileViewer />}
        </div>
        <div className="overflow-hidden">
          <ChatPanel registerHandle={registerChatHandle} />
        </div>
      </div>
    </div>
  );
}
