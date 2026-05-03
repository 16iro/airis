// PR 1 스캐폴딩 단계 — 빈 셸. 실제 UI는 PR 5에서 Welcome/Workspace/Settings로 분리.
function App() {
  return (
    <main className="flex min-h-full flex-col items-center justify-center gap-4 bg-background p-12 text-foreground">
      <h1 className="font-sans text-3xl font-semibold tracking-tight">
        airis
      </h1>
      <p className="text-muted-foreground">
        LLM 기반 교재 학습 보조 데스크톱 앱
      </p>
      <p className="rounded-md bg-muted px-3 py-1 font-mono text-xs text-muted-foreground">
        v0.1 scaffolding
      </p>
    </main>
  );
}

export default App;
