// 워크스페이스 — 3-pane 셸 (PR 32, prototype 100% 충실, D-070).
//
// [TopBar(48px)]
// [Sidebar(260px collapse) | BookViewer(flex) | ChatPanel(380px collapse)]
//
// 사이드바·챗 collapse 토글: Mod+B (사이드바), Mod+J (챗) — App.tsx에서 hookup.
// 사이드바가 닫히면 좌측 작은 floating chevron 버튼이 나타남 (열기). 챗도 동일.
// 슬라이드업 패널 영역(SlideupTabs)은 PR 33에서 추가.

import { ChevronRight } from "lucide-react";

import { BookViewer } from "@/components/BookViewer";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { Pane } from "@/components/layout/Pane";
import { StudySidebar } from "@/components/StudySidebar";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useUiStore } from "@/store/uiStore";

interface Props {
  registerChatHandle?: Parameters<typeof ChatPanel>[0]["registerHandle"];
}

export function Workspace({ registerChatHandle }: Props) {
  const activeBookId = useActiveBookStore((s) => s.bookId);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);
  const setSidebarOpen = useUiStore((s) => s.setSidebarOpen);
  const chatOpen = useUiStore((s) => s.chatOpen);
  const setChatOpen = useUiStore((s) => s.setChatOpen);

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <TopBar />
      <div className="relative flex min-h-0 flex-1">
        {sidebarOpen ? (
          <div className="w-[260px] shrink-0">
            <StudySidebar onClose={() => setSidebarOpen(false)} />
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="absolute left-2 top-2 z-10 h-7 w-7 p-0"
            onClick={() => setSidebarOpen(true)}
            aria-label="Open sidebar"
            title="사이드바 열기 (⌘B)"
          >
            <ChevronRight className="h-3 w-3" />
          </Button>
        )}

        <Pane className="min-w-0 flex-1">
          {activeBookId ? <BookViewer /> : <FileViewer />}
        </Pane>

        {chatOpen ? (
          <div className="w-[380px] shrink-0 border-l border-border">
            <ChatPanel registerHandle={registerChatHandle} />
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="absolute right-2 top-2 z-10 h-7 w-7 p-0"
            onClick={() => setChatOpen(true)}
            aria-label="Open chat"
            title="챗 열기 (⌘J)"
          >
            <ChevronRight className="h-3 w-3 rotate-180" />
          </Button>
        )}
      </div>
    </div>
  );
}
