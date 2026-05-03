// vitest 셋업 — 모든 테스트 파일 실행 전에 한 번 적용.
//
// @testing-library/jest-dom: `toBeInTheDocument`·`toHaveTextContent` 등 매처 등록.
// i18next: 컴포넌트가 useTranslation으로 키를 읽으니 부팅이 필요.
//   테스트에선 *키 그대로* 반환하는 모드(`returnEmptyString: false` + 빈 resources)
//   대신 실제 ko.json을 로드해 사용자 가시 텍스트를 그대로 검증한다.

import "@testing-library/jest-dom/vitest";

import i18next from "i18next";
import { initReactI18next } from "react-i18next";

import ko from "@/locales/ko.json";

if (!i18next.isInitialized) {
  void i18next.use(initReactI18next).init({
    lng: "ko",
    fallbackLng: "ko",
    resources: { ko: { translation: ko } },
    interpolation: { escapeValue: false },
    react: { useSuspense: false },
  });
}
