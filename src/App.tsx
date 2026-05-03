// 라우팅은 v0.1엔 단순 state 토글 — react-router 도입은 PR 5(스터디 도입)에서 검토.

import { useEffect, useState } from "react";

import { Settings } from "@/pages/Settings";
import { TopBar } from "@/components/TopBar";

type Route = "home" | "settings";

function App() {
  const [route, setRoute] = useState<Route>("home");

  // Mod+, 단축키 → Settings 토글. macOS=⌘ / Windows·Linux=Ctrl.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === ",") {
        e.preventDefault();
        setRoute((r) => (r === "settings" ? "home" : "settings"));
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (route === "settings") {
    return <Settings onClose={() => setRoute("home")} />;
  }

  return (
    <div className="flex min-h-full flex-col bg-background text-foreground">
      <TopBar onOpenSettings={() => setRoute("settings")} />
      <main className="flex flex-1 items-center justify-center p-12">
        <div className="space-y-4 text-center">
          <h1 className="text-3xl font-semibold tracking-tight">airis</h1>
          <p className="text-muted-foreground">
            LLM 기반 교재 학습 보조 데스크톱 앱
          </p>
          <p className="rounded-md bg-muted px-3 py-1 font-mono text-xs text-muted-foreground inline-block">
            v0.1 — PR 3 (Settings)
          </p>
          <p className="text-sm text-muted-foreground">
            우측 상단 ⚙ 버튼 또는 <kbd className="rounded border border-border px-1 font-mono text-xs">Ctrl/⌘ + ,</kbd> 로 설정 열기
          </p>
        </div>
      </main>
    </div>
  );
}

export default App;
