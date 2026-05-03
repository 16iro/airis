// eslint flat config — TypeScript + React 19 + react-hooks + react-refresh.
//
// 정책:
//   * 기본은 typescript-eslint "recommended" — 무경고 통과 필수.
//   * `unused-vars`는 `_`로 시작하는 식별자만 예외 허용 (의도적 미사용 표식).
//   * `no-explicit-any` 켜둠 — strict TS와 일관성. 정 필요하면 // eslint-disable-next-line.
//   * `react-hooks` 권장 + `react-refresh/only-export-components`로 HMR 안정성.
//
// 출처: AGENTS.md "TypeScript: strict + lint 권장 규칙".

import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";

export default tseslint.config(
  {
    // 본 프로젝트 코드만 검사 — design/·prototype/·.claude/는 git 제외 영역.
    ignores: [
      "dist",
      "node_modules",
      "src-tauri/target",
      "coverage",
      ".claude/**",
      "design/**",
      "prototype/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  {
    // shadcn/ui 컴포넌트는 cva variants 등을 *컴포넌트 외부*로 export하는 게 표준.
    // react-refresh 권고는 적용 어려워 해당 룰만 폴더 단위로 끔.
    files: ["src/components/ui/**/*.{ts,tsx}"],
    rules: { "react-refresh/only-export-components": "off" },
  },
  {
    // 테스트·셋업 파일은 jsdom + vitest globals 일부를 사용.
    files: ["src/**/*.{test,spec}.{ts,tsx}", "src/test/**/*.{ts,tsx}"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
  },
);
