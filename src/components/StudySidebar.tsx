// 좌측 TOC 사이드바 — prototype 100% 충실 (PR 32, D-070).
//
// 위에서 아래로:
//   1) 활성 스터디 메타 헤더 (이름)
//   2) 책 list — 주교재(필수 1권) + 부교재(N권). 책 클릭 시 펼침 + activeBookStore.open
//   3) 펼친 책의 헤딩 트리 — buildHeadingPlan으로 markdown source 파싱.
//      섹션 클릭 → activeBookStore.setSection (BookViewer 점프 + 백엔드 컨텍스트 박힘)
//
// 노드 상태 아이콘 5종 (passed/failed/active/goal/untouched) — 우리 백엔드는 active와
// untouched만 결정 가능 (passed/failed는 SRS·Recall 결과 누적 데이터 v0.4 이후).
// 일단 시각 컴포넌트는 5종 모두 렌더 가능하게 짜고, 데이터는 active/untouched만.

import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Play,
  Target,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Pane, PaneBody } from "@/components/layout/Pane";
import { Button } from "@/components/ui/button";
import { buildHeadingPlan } from "@/lib/headingPlan";
import type { BookEntry } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useBookStore } from "@/store/bookStore";
import { useStudyStore } from "@/store/studyStore";

type NodeState = "passed" | "failed" | "active" | "goal" | "untouched";

export function StudySidebar() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const books = useBookStore((s) => s.books);
  const refreshBooks = useBookStore((s) => s.refresh);
  const activeBookId = useActiveBookStore((s) => s.bookId);

  // 활성 스터디 변경 시 책 list 다시 로드.
  useEffect(() => {
    if (activeStudy) {
      void refreshBooks(activeStudy.slug);
    }
  }, [activeStudy, refreshBooks]);

  // role 기준 정렬: main 먼저, 그 다음 sub. 같은 role 내에서는 added_at desc.
  const sortedBooks = useMemo(() => {
    const main = books.filter((b) => b.role === "main");
    const sub = books.filter((b) => b.role === "sub");
    return [...main, ...sub];
  }, [books]);

  // PR 51: dockview 탭 헤더(아이콘+제목)와 TopBar 활성 스터디 칩이 같은 정보를 표시하므로
  // 사이드바 안의 TOC 헤더 + 활성 스터디 메타 영역을 제거. PaneBody만 남김.
  return (
    <Pane className="border-r border-border">
      <PaneBody>
        {sortedBooks.length === 0 ? (
          <p className="px-3 py-6 text-center text-xs text-muted-foreground">
            {t("sidebar.empty")}
          </p>
        ) : (
          <ul className="py-1">
            {sortedBooks.map((book) => (
              <li key={book.id}>
                <BookNode book={book} active={activeBookId === book.id} />
              </li>
            ))}
          </ul>
        )}
      </PaneBody>
    </Pane>
  );
}

function BookNode({ book, active }: { book: BookEntry; active: boolean }) {
  const { t } = useTranslation();
  const open_ = useActiveBookStore((s) => s.open);
  const close = useActiveBookStore((s) => s.close);
  const activeStudy = useStudyStore((s) => s.active);
  const content = useActiveBookStore((s) => s.content);
  const sectionPath = useActiveBookStore((s) => s.sectionPath);
  const setSection = useActiveBookStore((s) => s.setSection);

  const [userCollapsed, setUserCollapsed] = useState(false);
  // active로 전환되면 펼침. 사용자가 명시적으로 collapse하지 않은 한 active 동안 펼쳐 둠.
  const expanded = active && !userCollapsed;

  async function handleClick() {
    if (active) {
      setUserCollapsed((c) => !c);
      return;
    }
    if (!activeStudy) return;
    setUserCollapsed(false);
    try {
      await open_(activeStudy.slug, book.id);
    } catch (e) {
      console.warn("open book failed:", e);
    }
  }

  // 활성 책의 markdown source에서만 heading parse. 다른 책은 placeholder.
  const headings = useMemo(() => {
    if (!active || !content || content.format === "pdf") return [];
    return buildHeadingPlan(content.content);
  }, [active, content]);

  return (
    <div>
      <div
        className={cn(
          "group flex cursor-pointer items-center gap-1.5 px-2 py-1.5 text-[13px] hover:bg-muted",
          active && "bg-primary-soft text-primary",
        )}
        onClick={() => void handleClick()}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            void handleClick();
          }
        }}
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-90",
          )}
        />
        <span
          className={cn(
            "shrink-0 text-[10px] font-mono uppercase",
            book.role === "main"
              ? "text-primary"
              : "text-muted-foreground",
          )}
        >
          {book.role === "main"
            ? t("books.role_main")
            : t("books.role_sub")}
        </span>
        <span className="min-w-0 flex-1 truncate" title={book.title}>
          {book.title}
        </span>
        {active ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-5 w-5 shrink-0 p-0 opacity-0 group-hover:opacity-100"
            onClick={(e) => {
              e.stopPropagation();
              void close();
            }}
            aria-label={t("bookviewer.close")}
            title={t("bookviewer.close")}
          >
            <ChevronDown className="h-3 w-3 rotate-180" />
          </Button>
        ) : null}
      </div>

      {expanded && active ? (
        headings.length === 0 ? (
          <p className="px-6 py-2 text-[11px] text-muted-foreground">
            {content?.format === "pdf"
              ? t("sidebar.pdf_no_toc")
              : t("sidebar.no_headings")}
          </p>
        ) : (
          <ul className="pb-1">
            {headings.map((h) => {
              const state: NodeState =
                h.path === sectionPath ? "active" : "untouched";
              return (
                <li key={h.path}>
                  <div
                    className={cn(
                      "flex cursor-pointer items-center gap-1.5 px-2 py-1 text-[12px] hover:bg-muted",
                      state === "active" && "bg-primary-soft font-medium text-primary",
                    )}
                    style={{ paddingLeft: `${10 + h.level * 14}px` }}
                    onClick={() => void setSection(h.path)}
                    role="button"
                    tabIndex={0}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        void setSection(h.path);
                      }
                    }}
                  >
                    <NodeStateIcon state={state} />
                    <span className="min-w-0 flex-1 truncate" title={h.title}>
                      {h.title}
                    </span>
                  </div>
                </li>
              );
            })}
          </ul>
        )
      ) : null}
    </div>
  );
}

function NodeStateIcon({ state }: { state: NodeState }) {
  switch (state) {
    case "passed":
      return <CheckCircle2 className="h-3.5 w-3.5 text-[oklch(0.62_0.16_145)]" />;
    case "failed":
      return <AlertCircle className="h-3.5 w-3.5 text-[oklch(0.72_0.15_75)]" />;
    case "active":
      return <Play className="h-3.5 w-3.5 fill-current text-primary" />;
    case "goal":
      return <Target className="h-3.5 w-3.5 text-primary" />;
    case "untouched":
    default:
      return <Circle className="h-3.5 w-3.5 text-[oklch(0.85_0_0)]" />;
  }
}
