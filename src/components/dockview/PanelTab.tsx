// dockview 패널 탭 헤더 — 아이콘 + 제목 + close 버튼 (PR 50, D-070+).
//
// dockview의 DockviewReact `defaultTabComponent` prop에 전달.
// PANEL_ICONS 매핑으로 패널별 아이콘 표시. 제목은 api.title (변경 이벤트 구독).

import { X } from "lucide-react";
import { useEffect, useState } from "react";
import type { IDockviewPanelHeaderProps } from "dockview-react";

import { PANEL_ICONS } from "@/lib/panelIcons";
import type { DockPanelId } from "@/store/uiStore";

export function PanelTab({ api }: IDockviewPanelHeaderProps) {
  const Icon = PANEL_ICONS[api.id as DockPanelId];
  const [title, setTitle] = useState<string>(api.title ?? api.id);

  useEffect(() => {
    const sub = api.onDidTitleChange((event) => {
      setTitle(event.title);
    });
    return () => sub.dispose();
  }, [api]);

  return (
    <div className="flex h-full items-center gap-1.5 px-2.5 text-xs">
      {Icon ? <Icon size={12} className="shrink-0 opacity-70" /> : null}
      <span className="truncate">{title}</span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          api.close();
        }}
        className="-mr-1 ml-1 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-[var(--dv-icon-hover-background-color)] hover:text-foreground"
        aria-label="Close"
      >
        <X size={10} />
      </button>
    </div>
  );
}
