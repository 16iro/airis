// 단축키 도움말 모달 — `Mod+/`로 토글 (PR 36, D-070).

import { X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useUiStore } from "@/store/uiStore";

interface Shortcut {
  keys: string;
  labelKey: string;
}

const SHORTCUTS: Shortcut[] = [
  { keys: "⌘ /", labelKey: "shortcuts.toggle_help" },
  { keys: "⌘ ,", labelKey: "shortcuts.open_settings" },
  { keys: "⌘ B", labelKey: "shortcuts.toggle_sidebar" },
  { keys: "⌘ J", labelKey: "shortcuts.toggle_chat" },
  { keys: "⌘ ⇧ L", labelKey: "shortcuts.toggle_library" },
  { keys: "⌘ ⇧ W", labelKey: "shortcuts.go_workspace" },
  { keys: "⌘ 1", labelKey: "shortcuts.slideup_quiz" },
  { keys: "⌘ 2", labelKey: "shortcuts.slideup_notes" },
  { keys: "⌘ 3", labelKey: "shortcuts.slideup_srs" },
  { keys: "⌘ 4", labelKey: "shortcuts.slideup_progress" },
  { keys: "⌘ 5", labelKey: "shortcuts.slideup_memory" },
  { keys: "⌘ L", labelKey: "shortcuts.focus_chat_input" },
  { keys: "⌘ ↵", labelKey: "shortcuts.send_chat" },
];

export function ShortcutsDialog() {
  const { t } = useTranslation();
  const setOpen = useUiStore((s) => s.setShortcutsOpen);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="shortcuts-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={() => setOpen(false)}
    >
      <Card className="w-full max-w-md" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle id="shortcuts-title">
              {t("shortcuts.title")}
            </CardTitle>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setOpen(false)}
              aria-label={t("common.close")}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <ul className="space-y-1.5 text-sm">
            {SHORTCUTS.map((s) => (
              <li
                key={s.keys}
                className="flex items-center justify-between gap-2 py-1"
              >
                <span className="text-foreground">{t(s.labelKey)}</span>
                <span className="rounded border border-border bg-muted px-2 py-0.5 font-mono text-[11px] text-muted-foreground">
                  {s.keys}
                </span>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </div>
  );
}
