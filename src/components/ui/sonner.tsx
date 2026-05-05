// shadcn/ui 스타일의 Sonner 래퍼. App.tsx 한 곳에서 마운트한다.
//
// 테마는 settings.theme(`system|light|dark`)를 그대로 Sonner에 넘긴다.
// "system"은 Sonner 내부가 prefers-color-scheme로 자동 감지.

import { Toaster as SonnerToaster } from "sonner";

import { useSettingsStore } from "@/store/settingsStore";

export function Toaster() {
  const theme = useSettingsStore((s) => s.settings.theme);
  return (
    <SonnerToaster
      theme={theme}
      position="bottom-right"
      richColors
      closeButton
      duration={4000}
    />
  );
}
