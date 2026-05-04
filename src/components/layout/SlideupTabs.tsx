// 워크스페이스 중앙 영역 하단 5탭 — prototype 100% 충실 (PR 33, D-070).
//
// Quiz / Notes / SRS Deck / Progress / Memory.
// 탭 클릭 → SlideupPanel(320px)이 위로 슬라이드. 같은 탭 다시 클릭 시 닫힘.
// 단축키 Mod+1~5 (App.tsx에서 hookup).

import {
  Brain,
  ChartLine,
  Layers,
  ListChecks,
  Pencil,
} from "lucide-react";
import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { useUiStore, type SlideupTab } from "@/store/uiStore";

interface TabDef {
  id: SlideupTab;
  icon: ReactNode;
  labelKey: string;
  shortcut: string;
  countKey?: "srs" | "notes";
}

const TABS: TabDef[] = [
  { id: "quiz", icon: <ListChecks size={13} />, labelKey: "slideup.quiz", shortcut: "⌘1" },
  { id: "notes", icon: <Pencil size={13} />, labelKey: "slideup.notes", shortcut: "⌘2", countKey: "notes" },
  { id: "srs", icon: <Layers size={13} />, labelKey: "slideup.srs", shortcut: "⌘3", countKey: "srs" },
  { id: "progress", icon: <ChartLine size={13} />, labelKey: "slideup.progress", shortcut: "⌘4" },
  { id: "memory", icon: <Brain size={13} />, labelKey: "slideup.memory", shortcut: "⌘5" },
];

interface Props {
  /** 탭별 카운트 — undefined면 미표시. */
  counts?: Partial<Record<"srs" | "notes", number>>;
}

export function SlideupTabs({ counts }: Props) {
  const { t } = useTranslation();
  const slideupTab = useUiStore((s) => s.slideupTab);
  const setSlideupTab = useUiStore((s) => s.setSlideupTab);

  return (
    <div className="flex h-9 shrink-0 border-t border-border bg-card">
      {TABS.map((tab, i) => {
        const isActive = slideupTab === tab.id;
        const count =
          tab.countKey && counts ? counts[tab.countKey] : undefined;
        return (
          <button
            key={tab.id}
            type="button"
            onClick={() => setSlideupTab(isActive ? null : tab.id)}
            className={cn(
              "relative flex flex-1 items-center justify-center gap-1.5 border-r border-border text-xs font-medium transition-colors hover:bg-muted",
              i === TABS.length - 1 && "border-r-0",
              isActive
                ? "bg-primary-soft text-primary"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {isActive ? (
              <span className="absolute left-0 right-0 top-0 h-0.5 bg-primary" />
            ) : null}
            {tab.icon}
            <span>{t(tab.labelKey)}</span>
            {count != null && count > 0 ? (
              <span className="rounded-full bg-primary px-1.5 py-px text-[10px] font-semibold text-primary-foreground">
                {count}
              </span>
            ) : null}
            <span className="ml-1 hidden font-mono text-[10px] text-muted-foreground md:inline">
              {tab.shortcut}
            </span>
          </button>
        );
      })}
    </div>
  );
}
