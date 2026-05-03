// 라우팅·전역 단축키·테마 적용·drag-drop.
// v0.1: 'welcome' | 'workspace' | 'settings' 3개 페이지를 state로 토글.
// react-router 도입은 v0.2 (스터디 라우트 도입 시).

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef } from "react";

import { MemoryEditor } from "@/components/MemoryEditor";
import { PomodoroPanel } from "@/components/PomodoroPanel";
import { SrsPanel } from "@/components/SrsPanel";
import { Library } from "@/pages/Library";
import { NewStudyWizard } from "@/pages/NewStudyWizard";
import { Settings } from "@/pages/Settings";
import { Welcome } from "@/pages/Welcome";
import { Workspace } from "@/pages/Workspace";
import { useChatStore } from "@/store/chatStore";
import { useFileStore } from "@/store/fileStore";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

interface ChatPanelHandle {
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}

function App() {
  const page = useUiStore((s) => s.page);
  const setPage = useUiStore((s) => s.setPage);
  const memoryOpen = useUiStore((s) => s.memoryOpen);
  const setMemoryOpen = useUiStore((s) => s.setMemoryOpen);
  const pomodoroOpen = useUiStore((s) => s.pomodoroOpen);
  const setPomodoroOpen = useUiStore((s) => s.setPomodoroOpen);
  const srsOpen = useUiStore((s) => s.srsOpen);
  const setSrsOpen = useUiStore((s) => s.setSrsOpen);
  const settings = useSettingsStore((s) => s.settings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const loadSettings = useSettingsStore((s) => s.load);
  const fileOpen = useFileStore((s) => s.open);
  const activeStudy = useStudyStore((s) => s.active);
  const studyLoaded = useStudyStore((s) => s.loaded);
  const loadStudy = useStudyStore((s) => s.load);
  const hydrateChat = useChatStore((s) => s.hydrate);

  const chatHandleRef = useRef<ChatPanelHandle | null>(null);

  // 첫 마운트 — 백엔드에서 Settings·활성 스터디 병렬 로드.
  useEffect(() => {
    void loadSettings();
    void loadStudy();
  }, [loadSettings, loadStudy]);

  // 활성 스터디가 정해지면 챗 히스토리 hydrate.
  useEffect(() => {
    if (activeStudy) {
      void hydrateChat(activeStudy.slug);
    }
  }, [activeStudy, hydrateChat]);

  useEffect(() => {
    if (settingsLoaded) {
      setPage(settings.welcome_seen ? "workspace" : "welcome");
    }
  }, [settingsLoaded, settings.welcome_seen, setPage]);

  // 테마 적용 — settings.theme 변화 시 <html>.dark 토글.
  useThemeEffect(settings.theme);

  // 전역 단축키.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;

      if (e.key === ",") {
        e.preventDefault();
        setPage(page === "settings" ? "workspace" : "settings");
      } else if (e.key.toLowerCase() === "b") {
        e.preventDefault();
        setPage(page === "library" ? "workspace" : "library");
      } else if (e.key.toLowerCase() === "m" && activeStudy) {
        e.preventDefault();
        setMemoryOpen(!memoryOpen);
      } else if (e.key.toLowerCase() === "p" && e.shiftKey && activeStudy) {
        e.preventDefault();
        setPomodoroOpen(!pomodoroOpen);
      } else if (e.key.toLowerCase() === "k" && activeStudy) {
        e.preventDefault();
        setSrsOpen(!srsOpen);
      } else if (e.key.toLowerCase() === "l" && page === "workspace") {
        e.preventDefault();
        chatHandleRef.current?.inputRef.current?.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    page,
    setPage,
    memoryOpen,
    setMemoryOpen,
    pomodoroOpen,
    setPomodoroOpen,
    srsOpen,
    setSrsOpen,
    activeStudy,
  ]);

  // Drag-drop — Tauri 2 webview API. paths 받아 fileStore.open 호출.
  useEffect(() => {
    let unlisten: (() => void) | null = null;

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const paths = event.payload.paths;
          if (paths.length > 0) {
            void fileOpen(paths[0]);
            // 파일 받으면 워크스페이스로 이동.
            setPage("workspace");
          }
        }
      })
      .then((u) => {
        unlisten = u;
      });

    return () => {
      if (unlisten) unlisten();
    };
  }, [fileOpen, setPage]);

  if (!settingsLoaded || !studyLoaded) {
    return (
      <div className="flex min-h-full items-center justify-center bg-background text-muted-foreground">
        …
      </div>
    );
  }

  const pageContent =
    page === "settings" ? (
      <Settings />
    ) : page === "welcome" ? (
      <Welcome />
    ) : page === "library" ? (
      <Library />
    ) : page === "new-study" ? (
      <NewStudyWizard />
    ) : (
      <Workspace
        registerChatHandle={(h) => {
          chatHandleRef.current = h;
        }}
      />
    );

  return (
    <>
      {pageContent}
      {memoryOpen && activeStudy ? (
        <MemoryEditor onClose={() => setMemoryOpen(false)} />
      ) : null}
      {pomodoroOpen && activeStudy ? (
        <PomodoroPanel onClose={() => setPomodoroOpen(false)} />
      ) : null}
      {srsOpen && activeStudy ? (
        <SrsPanel onClose={() => setSrsOpen(false)} />
      ) : null}
    </>
  );
}

/**
 * settings.theme 변화 시 documentElement에 .dark 클래스 토글.
 * "system"이면 prefers-color-scheme 따름 + 변경 listener 등록.
 */
function useThemeEffect(theme: "system" | "light" | "dark") {
  useEffect(() => {
    const apply = (effective: "light" | "dark") => {
      document.documentElement.classList.toggle("dark", effective === "dark");
    };

    if (theme === "light" || theme === "dark") {
      apply(theme);
      return;
    }

    // system — OS 설정 추적.
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    apply(mq.matches ? "dark" : "light");
    const onChange = (e: MediaQueryListEvent) => apply(e.matches ? "dark" : "light");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);
}

export default App;
