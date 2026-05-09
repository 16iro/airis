// MetacogAlertToast — v0.5 PR 3 (D-100).
//
// metacog:alert 이벤트를 수신하면 Sonner toast로 우상단 알림 표시.
// 차단 X. 클릭(dismiss) 시 intervention_signal_dismiss 호출 + toast 닫기.
//
// BUG-002 패턴 (D-092): cancelled flag + .then(unlisten) 체이닝으로 race 방지.
// App.tsx 최상위에서 1회 마운트 — dockview 재마운트 시 listener 누수 없음.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { api } from "@/lib/api";
import { toast } from "@/lib/toast";
import type { MetacogAlert } from "@/lib/types";

/**
 * metacog:alert 이벤트 listener.
 * 렌더 출력 없음 — 이벤트 도착 시 Sonner toast를 동적으로 띄움.
 */
export function MetacogAlertToast() {
  const { t } = useTranslation();

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    void listen<MetacogAlert>("metacog:alert", (e) => {
      if (cancelled) return;
      const alert = e.payload;

      // 한국어 레이블로 조합 풀이.
      const combo = alert.signal_types
        .map((t) => signalTypeLabel(t))
        .join(" + ");

      const description = t("metacog.alert.body", { combo });

      // Sonner toast — info 톤. 클릭 시 dismiss + 닫기.
      toast.info(`${t("metacog.alert.title")}: ${description}`);

      // 각 signal_id dismiss (비동기, non-blocking).
      for (const signalId of alert.signal_ids) {
        api.interventionSignalDismiss(signalId).catch(() => {
          // dismiss 실패는 non-fatal — 다음 세션에서 다시 검출 가능.
        });
      }
    }).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten = u;
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
    // t(번역 함수)는 mount 시 고정 — 의존성 배열 포함.
  }, [t]);

  // 렌더 출력 없음 — Sonner Toaster가 App.tsx에 이미 있음.
  return null;
}

/** signal_type → 한국어 레이블 (ko.json 키와 동기화). */
function signalTypeLabel(signalType: string): string {
  const labels: Record<string, string> = {
    repeat_search: "같은 검색 반복",
    progress_recall_gap: "진도-회상 격차",
    self_report_low: "자기보고-실제 격차",
    short_dwell: "짧은 체류",
    forced_output_miss: "응답 후 정주행 X",
  };
  return labels[signalType] ?? signalType;
}
