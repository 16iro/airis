// 워크스페이스 — TopBar + BookList + (FileViewer + ChatPanel) 2-pane.
// v0.2 PR 11: BookList 추가. v0.2 PR 12부터 FileViewer → BookViewer 진화.

import { BookList } from "@/components/BookList";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { TopBar } from "@/components/TopBar";

interface Props {
  registerChatHandle?: Parameters<typeof ChatPanel>[0]["registerHandle"];
}

export function Workspace({ registerChatHandle }: Props) {
  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <TopBar />
      <BookList />
      <div className="grid flex-1 grid-cols-1 overflow-hidden md:grid-cols-[1fr_400px]">
        <div className="overflow-hidden border-r border-border">
          <FileViewer />
        </div>
        <div className="overflow-hidden">
          <ChatPanel registerHandle={registerChatHandle} />
        </div>
      </div>
    </div>
  );
}
