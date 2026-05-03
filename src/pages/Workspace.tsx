// 워크스페이스 — FileViewer (좌) + ChatPanel (우) 2-pane.
// v0.2부터 슬라이드업 패널·TOC 트리·Memory 패널 추가 예정.

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
