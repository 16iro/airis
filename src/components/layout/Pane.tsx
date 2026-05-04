// Pane / PaneHeader / PaneTitle / PaneBody — prototype의 .pane CSS 클래스와 1:1.
//
// 워크스페이스 3-pane(StudySidebar / BookViewer / ChatPanel)이 공유하는 셸.

import { type HTMLAttributes, type ReactNode } from "react";

import { cn } from "@/lib/utils";

export function Pane({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-col bg-card",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

export function PaneHeader({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex shrink-0 items-center justify-between gap-2 border-b border-border px-3.5 py-2.5",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

export function PaneTitle({ children }: { children: ReactNode }) {
  return (
    <span className="text-[12px] font-semibold uppercase tracking-[0.04em] text-muted-foreground">
      {children}
    </span>
  );
}

export function PaneBody({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("min-h-0 flex-1 overflow-auto", className)}
      {...rest}
    >
      {children}
    </div>
  );
}
