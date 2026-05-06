// v0.4.2 PR 3 — 비정상 종료 감지 알림 다이얼로그.
//
// 흐름:
//   1. 백엔드(`lib.rs::run` setup)가 앱 시작 직후 `index:abnormal_termination`
//      이벤트를 emit (status='running'으로 남은 indexing_jobs).
//   2. 본 컴포넌트가 App.tsx 최상위에서 listen 등록.
//   3. 이벤트 수신 시 "이전 인덱싱 비정상 종료 감지: N개 청크 남음. 재개?"
//      다이얼로그 노출.
//   4. 사용자 선택:
//      - "모두 재개" → 각 잡 `resumeIndexingJob`. 단 비정상 종료 잡은 worker 인스턴스가
//        없으니 *현재 PR에선 안내만 하고 사용자가 책 재인덱싱을 다시 누르도록 유도*.
//        (v0.4.4: 자동 worker 재기동.)
//      - "모두 취소" → 각 잡 `cancelIndexingJob` (DB status='cancelled').
//      - "닫기" → 다이얼로그만 닫고 잡은 그대로 (status='running'에 머무름 →
//        다음 시작 시 다시 알림).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AlertTriangle, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { api } from "@/lib/api";
import { toast } from "@/lib/toast";

interface AbnormalJob {
  job_id: number;
  book_id: string;
  tier: number;
  pending_chunks: number;
}

interface AbnormalPayload {
  jobs: AbnormalJob[];
}

export function AbnormalTerminationDialog() {
  const { t } = useTranslation();
  const [jobs, setJobs] = useState<AbnormalJob[] | null>(null);

  // BUG-002 (v0.4.4 PR 2, D-092): listener race 가드 — listen() Promise가 cleanup
  // 이후 resolve되면 unlisten이 null인 채 영구 누수. cancelled flag만으로는 핸들러
  // 호출은 막히지만 listener 자체는 살아남음 → 다음 mount에서 같은 이벤트 N회 처리.
  // .then(u) 시점에 cancelled면 즉시 u() 호출.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void listen<AbnormalPayload>("index:abnormal_termination", (e) => {
      if (cancelled) return;
      if (e.payload.jobs.length === 0) return;
      setJobs(e.payload.jobs);
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
  }, []);

  if (!jobs) return null;

  const totalChunks = jobs.reduce((acc, j) => acc + j.pending_chunks, 0);

  async function handleCancelAll() {
    if (!jobs) return;
    for (const j of jobs) {
      try {
        await api.cancelIndexingJob(j.job_id);
      } catch {
        // 워커 인스턴스 부재(=비정상 종료 후 메모리 상 worker 없음) → 무시. DB상 status는
        // 다음 시작 시 다시 잡힐 수 있지만, 사용자에게 *모두 취소* 의도는 충분히 전달.
      }
    }
    toast.success(t("books.abnormal_termination.cancel_all"));
    setJobs(null);
  }

  function handleDismiss() {
    setJobs(null);
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="abnormal-termination-title"
      className="fixed inset-0 z-[60] flex items-start justify-center overflow-y-auto bg-black/50 p-4 sm:items-center"
      onClick={handleDismiss}
    >
      <Card
        className="w-full max-w-md"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-2 border-b border-border px-5 py-3.5">
          <h2
            id="abnormal-termination-title"
            className="flex items-center gap-2 text-base font-semibold"
          >
            <AlertTriangle className="h-4 w-4 text-amber-500" />
            {t("books.abnormal_termination.title")}
          </h2>
          <Button
            variant="ghost"
            size="sm"
            onClick={handleDismiss}
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="space-y-3 px-5 py-4">
          <p className="text-sm">
            {t("books.abnormal_termination.body", {
              count: jobs.length,
              chunks: totalChunks,
            })}
          </p>
        </div>
        <div className="flex justify-end gap-2 border-t border-border px-5 py-3">
          <Button variant="outline" size="sm" onClick={handleDismiss}>
            {t("books.abnormal_termination.dismiss")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void handleCancelAll()}
          >
            {t("books.abnormal_termination.cancel_all")}
          </Button>
        </div>
      </Card>
    </div>
  );
}
