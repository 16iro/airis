// RecallAutoTrigger — v0.5 PR 4 (D-101).
//
// recall:auto_trigger 이벤트를 수신하면 RecallChallengeDialog를 모달로 표시.
// 동시에 한 개만 열리도록 큐 없이 단순 replace(새 이벤트가 오면 이전 챌린지는 dismissed).
//
// BUG-002 패턴(D-092): cancelled flag + .then(unlisten) 체이닝으로 race 방지.
// App.tsx 최상위에서 1회 마운트 — listener 누수 없음.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

import { RecallChallengeDialog } from "@/components/RecallChallengeDialog";
import { api } from "@/lib/api";
import type { RecallChallenge, RecallChallengeSpec, RecallOutcome } from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";

interface PendingChallenge {
  studySlug: string;
  challenge: RecallChallenge;
}

/**
 * recall:auto_trigger 이벤트 listener.
 * 이벤트 도착 시 challenge를 생성해 모달로 띄운다.
 */
export function RecallAutoTrigger() {
  const activeStudy = useStudyStore((s) => s.active);
  const settings = useSettingsStore((s) => s.settings);
  const [pending, setPending] = useState<PendingChallenge | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    void listen<RecallChallengeSpec>("recall:auto_trigger", (e) => {
      if (cancelled) return;
      if (!activeStudy) return;
      if (!settings.learning_recall_auto_trigger) return;

      const spec = e.payload;

      // 챌린지 생성 (async, fire-and-forget — 실패하면 조용히 무시).
      void api
        .recallGenerateChallenge(
          activeStudy.slug,
          spec.chunk_id,
          settings.learning_recall_strength,
        )
        .then((challenge) => {
          if (cancelled) return;
          // 이전에 열린 챌린지가 있으면 새 것으로 대체 (구 챌린지는 dismissed).
          setPending({ studySlug: activeStudy.slug, challenge });
        })
        .catch(() => {
          // 생성 실패 — 조용히 무시.
        });
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
  }, [activeStudy, settings.learning_recall_auto_trigger, settings.learning_recall_strength]);

  function handleClose(_outcome: RecallOutcome) {
    setPending(null);
  }

  if (!pending) return null;

  return (
    <RecallChallengeDialog
      studySlug={pending.studySlug}
      challenge={pending.challenge}
      onClose={handleClose}
    />
  );
}
