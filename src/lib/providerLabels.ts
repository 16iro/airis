// v0.4.4.x followup §1.3 — provider id → 표시 이름·강조 색·아바타 톤 매핑.
//
// 시스템 톤(sky/orange/lime)을 그대로 가져와 라벨에 강조 색만 입히고, 아바타 배경은
// 같은 색조의 약한 변형으로 통일해 다른 UI(컨텍스트 칩 등)와 시각적 일관성 유지.
// 공식 로고 SVG는 v0.5+로 미루고 *텍스트 라벨 + 색* 으로 시작.

export interface ProviderDisplay {
  /** UI에 그대로 출력할 이름. */
  label: string;
  /** 발신자 라벨 옆 색깔. tailwind class. */
  labelClass: string;
  /** 아바타 동그라미 배경. tailwind class — 어두운 모드에서도 충분히 대비되는 톤. */
  avatarClass: string;
}

const FALLBACK: ProviderDisplay = {
  label: "Assistant",
  labelClass: "text-muted-foreground",
  avatarClass: "bg-accent",
};

/**
 * provider id → 표시 정보. 미지의 id 또는 null/undefined면 "Assistant" 폴백.
 *
 * - `anthropic` / `claude` (CLI 별칭) → "Claude" + sky 톤.
 * - `openai` / `codex` (CLI 별칭) → "ChatGPT" + lime 톤.
 * - `gemini` (HTTP·CLI 동일 id) → "Gemini" + orange 톤.
 * - `mock` (테스트) → "Mock" + 무채색.
 */
export function providerDisplay(provider: string | null | undefined): ProviderDisplay {
  switch (provider) {
    case "anthropic":
    case "claude":
      return {
        label: "Claude",
        labelClass: "text-sky-600 dark:text-sky-400",
        avatarClass: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
      };
    case "openai":
    case "codex":
      return {
        label: "ChatGPT",
        labelClass: "text-lime-600 dark:text-lime-400",
        avatarClass: "bg-lime-500/15 text-lime-700 dark:text-lime-300",
      };
    case "gemini":
      return {
        label: "Gemini",
        labelClass: "text-orange-600 dark:text-orange-400",
        avatarClass: "bg-orange-500/15 text-orange-700 dark:text-orange-300",
      };
    case "mock":
      return {
        label: "Mock",
        labelClass: "text-muted-foreground",
        avatarClass: "bg-muted text-muted-foreground",
      };
    default:
      return FALLBACK;
  }
}
