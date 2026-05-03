// F14 인앱 업데이트 다이얼로그.
// 앱 시작 시 1회 + 24h throttle (App.tsx hook). 새 버전 있으면 표시.

import { ExternalLink, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { UpdateInfo } from "@/lib/types";

interface Props {
  info: UpdateInfo;
  onClose: () => void;
}

export function UpdateDialog({ info, onClose }: Props) {
  const { t } = useTranslation();

  function openRelease() {
    void import("@tauri-apps/plugin-opener").then(({ openUrl }) => {
      void openUrl(info.release_url);
    });
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <Card className="w-full max-w-md" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <CardTitle id="update-title">{t("update.title")}</CardTitle>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2"
              onClick={onClose}
            >
              <X size={14} />
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <div>
              <p className="text-muted-foreground">{t("update.current_label")}</p>
              <p className="font-mono">{info.current}</p>
            </div>
            <div>
              <p className="text-muted-foreground">{t("update.latest_label")}</p>
              <p className="font-mono font-medium text-primary">{info.latest}</p>
            </div>
          </div>
          {info.body ? (
            <pre className="max-h-48 overflow-y-auto rounded-md bg-muted p-2 text-[11px] whitespace-pre-wrap">
              {info.body.slice(0, 800)}
              {info.body.length > 800 ? "…" : ""}
            </pre>
          ) : null}
          {info.has_sha256 ? (
            <p className="text-[11px] text-muted-foreground">
              {t("update.sha256_note")}
            </p>
          ) : null}
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="outline" size="sm" onClick={onClose}>
              {t("update.later")}
            </Button>
            <Button size="sm" onClick={openRelease}>
              <ExternalLink size={14} />
              {t("update.open_release")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
