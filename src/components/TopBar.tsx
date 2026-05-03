// 상단 바 — 좌측 로고 + 우측 Settings 아이콘.
// PR 5에서 다크/라이트 토글·스터디 셀렉터 등이 추가될 예정.

import { Settings as SettingsIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

interface Props {
  onOpenSettings: () => void;
}

export function TopBar({ onOpenSettings }: Props) {
  return (
    <header className="flex h-12 items-center justify-between border-b border-border bg-background px-4">
      <span className="font-semibold tracking-tight">airis</span>
      <Button
        variant="ghost"
        size="sm"
        onClick={onOpenSettings}
        aria-label="설정 열기"
        title="설정 (⌘,)"
      >
        <SettingsIcon size={18} />
      </Button>
    </header>
  );
}
