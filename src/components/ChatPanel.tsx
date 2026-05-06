// 챗 패널 — 입력 + 스트리밍 + 메시지 히스토리.
// Tauri events 구독: chat:chunk·chat:done·chat:error.
// 단축키: Mod+L → 입력 포커스, Mod+Enter → 전송 (App.tsx에서 처리).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FlaskConical, Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { AbComparePanel } from "@/components/AbComparePanel";
import { ChatMessage } from "@/components/ChatMessage";
import { TriggerDialog } from "@/components/TriggerDialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type TriggerHit,
  type Usage,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { useChatStore } from "@/store/chatStore";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

interface ChunkPayload {
  handle: string;
  text: string;
}
interface DonePayload {
  handle: string;
  usage: Usage;
}
interface ErrorPayload {
  handle: string;
  error: { kind: string; message?: string };
  job_id: number | null;
}
interface ViolationPayload {
  handle: string;
  violations: import("@/lib/types").ViolationHit[];
}
interface ContextPayload {
  handle: string;
  context: import("@/lib/types").ChatContextSummary;
}

interface ChatPanelHandle {
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}

export function ChatPanel({
  registerHandle,
}: {
  registerHandle?: (h: ChatPanelHandle) => void;
}) {
  const { t } = useTranslation();
  const [input, setInput] = useState("");
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [pendingTrigger, setPendingTrigger] = useState<TriggerHit | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // v0.4.1 PR 5 — dev 토글 ON일 때 사용자가 A/B 모드로 진입할 수 있다. 디폴트 OFF.
  const devAbCompare = useSettingsStore((s) => s.settings.dev_ab_compare);
  // v0.4.4 PR 2 (D-092) — dev raw event log 토글. ON이면 chat:* 이벤트마다 카운터+payload
  // 콘솔 출력 (BUG-002 같은 listener 누수 회귀 디버깅).
  const devEventLog = useSettingsStore((s) => s.settings.dev_event_log);
  const [abMode, setAbMode] = useState(false);
  // 토글 자체가 OFF로 바뀌면 모드도 끔 (보호 차원).
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (!devAbCompare && abMode) setAbMode(false);
  }, [devAbCompare, abMode]);

  const messages = useChatStore((s) => s.messages);
  const streamingHandle = useChatStore((s) => s.streamingHandle);
  const addUserMessage = useChatStore((s) => s.addUserMessage);
  const beginAssistantStream = useChatStore((s) => s.beginAssistantStream);
  const appendChunk = useChatStore((s) => s.appendChunk);
  const finalizeStream = useChatStore((s) => s.finalizeStream);
  const failStream = useChatStore((s) => s.failStream);
  const attachViolations = useChatStore((s) => s.attachViolations);
  const attachContext = useChatStore((s) => s.attachContext);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const activeStudy = useStudyStore((s) => s.active);
  const activeProvider = useSettingsStore((s) => s.settings.active_provider);
  const authMode = useSettingsStore((s) => s.settings.auth_mode);
  const interventionLevel = useSettingsStore(
    (s) => s.settings.intervention_level,
  );

  // PR 28 — 챗 가능 여부 = (auth_mode=cli) OR (auth_mode=api_key && 키 보유).
  // CLI 모드는 OAuth가 CLI 자체에서 처리되므로 keyring 체크 의미 없음.
  // 프로바이더별 CLI 인증 상태 검증은 백엔드 chat_send + cli_auth_status_*에 위임 (UI는 fail-fast 안 함).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      if (authMode === "cli") {
        if (!cancelled) setHasKey(true);
        return;
      }
      try {
        const present = await api.apiKeyPresent(activeProvider);
        if (!cancelled) setHasKey(present);
      } catch {
        if (!cancelled) setHasKey(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [streamingHandle, activeProvider, authMode]); // 키 추가/삭제 + 프로바이더 전환 + auth_mode 전환 시 재확인.

  // 부모(App)가 단축키 처리에 사용할 input ref 등록.
  useEffect(() => {
    if (registerHandle) registerHandle({ inputRef });
  }, [registerHandle]);

  // 워크스페이스 단축키(⌘L)가 dockview 안에서 input ref에 직접 닿지 못해 CustomEvent로 위임받음.
  useEffect(() => {
    const handler = () => inputRef.current?.focus();
    window.addEventListener("airis:focus-chat-input", handler);
    return () => window.removeEventListener("airis:focus-chat-input", handler);
  }, []);

  // Tauri events 구독.
  //
  // BUG-001/002 (v0.4.4 PR 1, D-091): `listen()`은 비동기라 listener 등록이 완료되기
  // 전에 컴포넌트가 unmount되면 (StrictMode dev 빌드의 mount/unmount/mount 사이클,
  // dockview 패널 재마운트 등) cleanup이 *이미 비어있는* unlisteners 배열만 비우고
  // 끝나버린다. 이후 Promise가 resolve돼서 unlisteners.push가 실행되어도 cleanup은
  // 이미 지나간 상태라 listener가 *영구적으로* 살아남아 다음 mount의 listener와
  // 함께 같은 이벤트를 N회 처리한다. → chat:chunk가 N번 append → 누적 prefix 또는
  // 응답 N회 반복.
  //
  // fix: cleanup 시 cancelled flag를 켜고 *모든* 등록 Promise에 .then(unlisten)을
  // 체이닝해 cleanup 이후 도착한 listener도 즉시 해제. 등록 순서와 무관하게 안전.
  useEffect(() => {
    let cancelled = false;
    const settled: UnlistenFn[] = [];
    // dev_event_log ON 시 이벤트별 카운터 — listener 누수 시 같은 event가 N>1로 빠르게 증가.
    const counters = { chunk: 0, done: 0, violation: 0, context: 0, error: 0 };

    function track(p: Promise<UnlistenFn>) {
      void p.then((u) => {
        if (cancelled) {
          // 이미 cleanup 지났음 — 곧장 해제.
          u();
        } else {
          settled.push(u);
        }
      });
    }

    track(
      listen<ChunkPayload>("chat:chunk", (event) => {
        if (devEventLog) {
          counters.chunk += 1;
          console.debug("chat:chunk", { count: counters.chunk, payload: event.payload });
        }
        appendChunk(event.payload.handle, event.payload.text);
      }),
    );

    track(
      listen<DonePayload>("chat:done", (event) => {
        if (devEventLog) {
          counters.done += 1;
          console.debug("chat:done", { count: counters.done, payload: event.payload });
        }
        finalizeStream(event.payload.handle, event.payload.usage);
      }),
    );

    track(
      listen<ViolationPayload>("chat:violation", (event) => {
        if (devEventLog) {
          counters.violation += 1;
          console.debug("chat:violation", {
            count: counters.violation,
            payload: event.payload,
          });
        }
        attachViolations(event.payload.handle, event.payload.violations);
      }),
    );

    track(
      listen<ContextPayload>("chat:context", (event) => {
        if (devEventLog) {
          counters.context += 1;
          console.debug("chat:context", {
            count: counters.context,
            payload: event.payload,
          });
        }
        attachContext(event.payload.handle, event.payload.context);
      }),
    );

    track(
      listen<ErrorPayload>("chat:error", (event) => {
        if (devEventLog) {
          counters.error += 1;
          console.debug("chat:error", { count: counters.error, payload: event.payload });
        }
        const errMessage =
          event.payload.error.message ?? `(${event.payload.error.kind})`;
        failStream(
          event.payload.handle,
          errMessage,
          event.payload.job_id ?? undefined,
        );
      }),
    );

    return () => {
      cancelled = true;
      for (const u of settled) u();
    };
  }, [
    appendChunk,
    finalizeStream,
    failStream,
    attachViolations,
    attachContext,
    devEventLog,
  ]);

  // 새 메시지 들어올 때 자동 스크롤.
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  async function handleSend() {
    const trimmed = input.trim();
    if (!trimmed || streamingHandle) return;
    if (hasKey === false) {
      setSettingsOpen(true);
      return;
    }
    if (!activeStudy) {
      // 부팅 hydration이 끝나기 전 — 사용자가 마구 enter 치는 케이스. 무시.
      return;
    }

    addUserMessage(trimmed);
    setInput("");

    // 트리거 감지 — intervention_level=off가 아닌 경우. 사용자 발화에서 추출.
    if (interventionLevel !== "off") {
      void detectAndApplyTriggers(
        trimmed,
        activeStudy.slug,
        interventionLevel,
        setPendingTrigger,
      );
    }

    try {
      const { handle } = await api.chatSend(activeStudy.slug, trimmed, null);
      beginAssistantStream(handle);
    } catch (e) {
      const errMessage = isAppError(e)
        ? appErrorMessage(e)
        : String(e);
      // 사용자 메시지 직후라 별도 어시스턴트 메시지를 못 만들었음 → 일단 사용자에게 alert.
      addUserMessageFailure(errMessage);
    }
  }

  function addUserMessageFailure(msg: string) {
    // 간단히 — chatStore에 새 어시스턴트 메시지 추가 후 곧장 fail.
    const handle = `local-fail-${Date.now()}`;
    beginAssistantStream(handle);
    failStream(handle, msg);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === "Enter") {
      e.preventDefault();
      void handleSend();
    }
  }

  // dev 토글이 켜져 있을 때만 보이는 A/B 모드 진입 chip + 모드 분기.
  // 토글 OFF면 컴포넌트(AbComparePanel) 자체 렌더 X — handoff §1.3 게이트.
  if (devAbCompare && abMode) {
    return (
      <div className="flex h-full flex-col">
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border px-3 py-1.5">
          <button
            type="button"
            onClick={() => setAbMode(false)}
            className="flex items-center gap-1.5 rounded-md border border-border bg-card px-2 py-1 text-[11px] hover:border-border-strong"
            aria-label={t("ab_compare.exit")}
          >
            <FlaskConical size={12} />
            <span>{t("ab_compare.exit")}</span>
          </button>
        </div>
        <div className="min-h-0 flex-1">
          <AbComparePanel />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {hasKey === false ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
            <h3 className="text-lg font-medium">{t("chat.no_api_key")}</h3>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("chat.no_api_key_hint")}
            </p>
            <Button onClick={() => setSettingsOpen(true)}>
              {t("chat.open_settings")}
            </Button>
          </div>
        ) : messages.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
            <h3 className="text-lg font-medium">{t("chat.empty_title")}</h3>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("chat.empty_hint")}
            </p>
          </div>
        ) : (
          <div className="divide-y divide-border">
            {messages.map((m) => (
              <ChatMessage key={m.id} message={m} />
            ))}
          </div>
        )}
      </div>
      <div className="border-t border-border p-3">
        {devAbCompare ? (
          <div className="mb-2">
            <button
              type="button"
              onClick={() => setAbMode(true)}
              className={cn(
                "inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] transition-colors",
                "border-primary/30 bg-primary/5 text-primary hover:bg-primary/10",
              )}
              aria-label={t("ab_compare.enter")}
            >
              <FlaskConical size={12} />
              <span>{t("ab_compare.enter")}</span>
            </button>
          </div>
        ) : null}
        <div className="flex items-end gap-2">
          <Textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t("chat.input_placeholder")}
            rows={2}
            disabled={streamingHandle !== null}
            className="flex-1 resize-none font-sans"
          />
          <Button
            onClick={() => void handleSend()}
            disabled={!input.trim() || streamingHandle !== null}
            size="sm"
            aria-label={t("chat.send")}
          >
            <Send size={16} />
          </Button>
        </div>
      </div>

      {pendingTrigger && activeStudy ? (
        <TriggerDialog
          studySlug={activeStudy.slug}
          hit={pendingTrigger}
          onClose={() => setPendingTrigger(null)}
        />
      ) : null}
    </div>
  );
}

/**
 * 사용자 발화에서 트리거 감지 → intervention_level에 따라:
 * - confirm: 첫 hit를 다이얼로그로 (현재는 1개씩만, 큐는 v0.3+)
 * - auto: 모든 hit를 즉시 Memory에 적용
 */
async function detectAndApplyTriggers(
  text: string,
  studySlug: string,
  level: "confirm" | "auto",
  setPending: (h: TriggerHit | null) => void,
) {
  try {
    const hits = await api.memoryDetectTriggers(text);
    if (hits.length === 0) return;
    if (level === "auto") {
      for (const h of hits) {
        await api.memoryApplyTrigger(studySlug, h);
      }
    } else {
      // confirm: 가장 먼저 잡힌 hit만 다이얼로그. 사용자 결정 후 다음.
      setPending(hits[0]);
    }
  } catch (e) {
    console.error("trigger detection failed:", e);
  }
}
