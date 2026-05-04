// 라우팅·전역 단축키·테마 적용·drag-drop.
// v0.1: 'welcome' | 'workspace' | 'settings' 3개 페이지를 state로 토글.
// react-router 도입은 v0.2 (스터디 라우트 도입 시).

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef, useState } from "react";

import { RecallPanel } from "@/components/RecallPanel";
import { SrsPanel } from "@/components/SrsPanel";
import { UpdateDialog } from "@/components/UpdateDialog";
import { api } from "@/lib/api";
import type { UpdateInfo } from "@/lib/types";
import { Library } from "@/pages/Library";
import { NewStudyDialog } from "@/components/NewStudyDialog";
import { ShortcutsDialog } from "@/components/ShortcutsDialog";
import { Settings } from "@/pages/Settings";
import { Welcome } from "@/pages/Welcome";
import { Workspace } from "@/pages/Workspace";
import { useChatStore } from "@/store/chatStore";
import { useFileStore } from "@/store/fileStore";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

const UPDATE_THROTTLE_MS = 24 * 60 * 60 * 1000; // 24h
const UPDATE_LAST_CHECK_KEY = "airis:update:last_check";
const QUEUE_POLL_MS = 30 * 1000; // 30s

interface ChatPanelHandle {
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}

function App() {
  const page = useUiStore((s) => s.page);
  const setPage = useUiStore((s) => s.setPage);
  // Memory는 PR 33에서 SlideupPanel(Mod+5)로 흡수됨.
  const newStudyOpen = useUiStore((s) => s.newStudyOpen);
  // Pomodoro는 PR 34에서 TopBar 인라인(PomodoroInline)으로 흡수됨.
  const srsOpen = useUiStore((s) => s.srsOpen);
  const setSrsOpen = useUiStore((s) => s.setSrsOpen);
  const recallOpen = useUiStore((s) => s.recallOpen);
  const setRecallOpen = useUiStore((s) => s.setRecallOpen);
  const settings = useSettingsStore((s) => s.settings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const loadSettings = useSettingsStore((s) => s.load);
  const fileOpen = useFileStore((s) => s.open);
  const activeStudy = useStudyStore((s) => s.active);
  const studyLoaded = useStudyStore((s) => s.loaded);
  const loadStudy = useStudyStore((s) => s.load);
  const hydrateChat = useChatStore((s) => s.hydrate);

  const chatHandleRef = useRef<ChatPanelHandle | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);

  // F14 — 앱 시작 시 1회 + 24h throttle.
  useEffect(() => {
    const last = parseInt(localStorage.getItem(UPDATE_LAST_CHECK_KEY) ?? "0", 10);
    if (Date.now() - last < UPDATE_THROTTLE_MS) return;
    localStorage.setItem(UPDATE_LAST_CHECK_KEY, String(Date.now()));
    void api.checkForUpdate().then((info) => {
      if (info) setUpdateInfo(info);
    }).catch((e) => console.warn("update check failed:", e));
  }, []);

  // 자동 큐 워커 — 30초 주기. due 잡을 받아 retry, 결과는 console만 (UI는 chat:done이 처리).
  useEffect(() => {
    let cancelled = false;
    async function tick() {
      if (cancelled) return;
      try {
        const due = await api.listDueJobs();
        for (const job of due) {
          if (cancelled) return;
          await api.retryFailedJob(job.id).catch((e) => {
            console.warn("auto retry failed:", e);
          });
        }
      } catch (e) {
        console.warn("queue worker poll failed:", e);
      }
    }
    void tick();
    const id = setInterval(() => void tick(), QUEUE_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

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

  // Density attribute — uiStore.density → <html data-density="...">.
  const density = useUiStore((s) => s.density);
  useEffect(() => {
    document.documentElement.setAttribute("data-density", density);
  }, [density]);

  // Accent hue — uiStore.accentHue → <html style="--accent-h: ...">.
  const accentHue = useUiStore((s) => s.accentHue);
  useEffect(() => {
    document.documentElement.style.setProperty("--accent-h", String(accentHue));
  }, [accentHue]);

  // 전역 단축키 — 워크스페이스 내부 단축키(Mod+B/J/1~5/L)는 dockview 도입 후
  // Workspace 컴포넌트가 직접 처리한다. App.tsx는 페이지·모달·라우팅 단축키만.
  const shortcutsOpen = useUiStore((s) => s.shortcutsOpen);
  const setShortcutsOpen = useUiStore((s) => s.setShortcutsOpen);
  const settingsOpen = useUiStore((s) => s.settingsOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;

      if (e.key === ",") {
        e.preventDefault();
        setSettingsOpen(!settingsOpen);
      } else if (e.key === "/") {
        e.preventDefault();
        setShortcutsOpen(true);
      } else if (e.shiftKey && e.key.toLowerCase() === "l") {
        e.preventDefault();
        setPage(page === "library" ? "workspace" : "library");
      } else if (e.shiftKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        if (activeStudy) setPage("workspace");
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    page,
    setPage,
    activeStudy,
    setShortcutsOpen,
    settingsOpen,
    setSettingsOpen,
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
    page === "welcome" ? (
      <Welcome />
    ) : page === "library" ? (
      <Library />
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
      {/* Memory는 PR 33, Pomodoro는 PR 34 인라인으로 흡수. SRS/Recall modal은 slideup의 시작 버튼으로만 트리거. */}
      {srsOpen && activeStudy ? (
        <SrsPanel onClose={() => setSrsOpen(false)} />
      ) : null}
      {recallOpen && activeStudy ? (
        <RecallPanel onClose={() => setRecallOpen(false)} />
      ) : null}
      {newStudyOpen ? <NewStudyDialog /> : null}
      {settingsOpen ? <Settings /> : null}
      {shortcutsOpen ? <ShortcutsDialog /> : null}
      {updateInfo ? (
        <UpdateDialog info={updateInfo} onClose={() => setUpdateInfo(null)} />
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
