// 패널별 아이콘 매핑 — TopBar 토글과 dockview 탭 헤더가 공유 (PR 50, D-070+).

import {
  BookOpen,
  Brain,
  ChartLine,
  Layers,
  List,
  ListChecks,
  MessageSquare,
  Pencil,
  Timer,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import type { DockPanelId } from "@/store/uiStore";

export const PANEL_ICONS: Record<DockPanelId, LucideIcon> = {
  toc: List,
  viewer: BookOpen,
  chat: MessageSquare,
  quiz: ListChecks,
  notes: Pencil,
  srs: Layers,
  progress: ChartLine,
  memory: Brain,
  pomodoro: Timer,
};
