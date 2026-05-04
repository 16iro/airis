// 워크스페이스 — 3-pane 셸 + bottom-sheet (PR 32~33, prototype 100% 충실, D-070).
//
// [TopBar(48px)]
// [Sidebar(260px collapse) | Center(flex) | ChatPanel(380px collapse)]
//
// Center = BookViewer/FileViewer + SlideupTabs(36px) + SlideupPanel(absolute 320px when active).

import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { BookViewer } from "@/components/BookViewer";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { Pane } from "@/components/layout/Pane";
import { SlideupPanel } from "@/components/layout/SlideupPanel";
import { SlideupTabs } from "@/components/layout/SlideupTabs";
import { MemoryPanelContent } from "@/components/MemoryPanelContent";
import { StudySidebar } from "@/components/StudySidebar";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useUiStore } from "@/store/uiStore";

interface Props {
  registerChatHandle?: Parameters<typeof ChatPanel>[0]["registerHandle"];
}

export function Workspace({ registerChatHandle }: Props) {
  const { t } = useTranslation();
  const activeBookId = useActiveBookStore((s) => s.bookId);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);
  const setSidebarOpen = useUiStore((s) => s.setSidebarOpen);
  const chatOpen = useUiStore((s) => s.chatOpen);
  const setChatOpen = useUiStore((s) => s.setChatOpen);
  const slideupTab = useUiStore((s) => s.slideupTab);

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
            aria-label={t("workspace.open_sidebar")}
            title={t("workspace.open_sidebar_tooltip")}
          >
            <ChevronRight className="h-3 w-3" />
          </Button>
        )}

        <Pane className="relative min-w-0 flex-1">
          <div className="min-h-0 flex-1 overflow-hidden">
            {activeBookId ? <BookViewer /> : <FileViewer />}
          </div>
          <SlideupPanel title={slideupTab ? t(`slideup.${slideupTab}`) : undefined}>
            {slideupTab === "memory" ? <MemoryPanelContent /> : null}
            {slideupTab === "quiz" ? <QuizPlaceholder /> : null}
            {slideupTab === "notes" ? <NotesPlaceholder /> : null}
            {slideupTab === "srs" ? <SrsPlaceholder /> : null}
            {slideupTab === "progress" ? <ProgressPlaceholder /> : null}
          </SlideupPanel>
          <SlideupTabs />
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
            aria-label={t("workspace.open_chat")}
            title={t("workspace.open_chat_tooltip")}
          >
            <ChevronRight className="h-3 w-3 rotate-180" />
          </Button>
        )}
      </div>
    </div>
  );
}

function QuizPlaceholder() {
  const { t } = useTranslation();
  return <p className="text-xs text-muted-foreground">{t("slideup.quiz_placeholder")}</p>;
}

function NotesPlaceholder() {
  const { t } = useTranslation();
  return <p className="text-xs text-muted-foreground">{t("slideup.notes_placeholder")}</p>;
}

function SrsPlaceholder() {
  const { t } = useTranslation();
  return <p className="text-xs text-muted-foreground">{t("slideup.srs_placeholder")}</p>;
}

function ProgressPlaceholder() {
  const { t } = useTranslation();
  return <p className="text-xs text-muted-foreground">{t("slideup.progress_placeholder")}</p>;
}
