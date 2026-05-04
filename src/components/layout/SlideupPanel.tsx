// 워크스페이스 bottom-sheet — prototype 100% 충실 (PR 33, D-070).
//
// SlideupTabs에서 활성 탭 콘텐츠를 표시. height 320px, BookViewer 영역 안의 absolute.
// 부모 컨테이너가 relative이어야 함. SlideupTabs 위에 깔린다 (bottom: 36px).

import { X } from "lucide-react";
import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useUiStore } from "@/store/uiStore";

interface Props {
  children?: ReactNode;
  /** 탭 라벨 — 헤더에 표시. */
  title?: string;
}

export function SlideupPanel({ children, title }: Props) {
  const { t } = useTranslation();
  const slideupTab = useUiStore((s) => s.slideupTab);
  const setSlideupTab = useUiStore((s) => s.setSlideupTab);

  if (!slideupTab) return null;

  return (
    <div
      className="absolute bottom-9 left-0 right-0 z-[var(--z-slideup)] h-80 border-t border-border bg-card shadow-[0_-8px_24px_-12px_rgb(0_0_0_/_0.15)]"
      style={{ animation: "slideUp 240ms cubic-bezier(0.16, 1, 0.3, 1)" }}
    >
      <div className="flex items-center justify-between border-b border-border px-4 py-2.5">
        <span className="text-[13px] font-semibold capitalize">
          {title ?? slideupTab}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 w-6 p-0"
          onClick={() => setSlideupTab(null)}
          aria-label={t("common.close")}
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
      <div className="h-[calc(100%-41px)] overflow-auto p-4">{children}</div>
    </div>
  );
}
