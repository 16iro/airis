// react-i18next 셋업.
// v0.1: 한국어만. v0.2에 en.json 추가 예정.

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import ko from "@/locales/ko.json";

void i18n.use(initReactI18next).init({
  resources: {
    ko: { translation: ko },
  },
  lng: "ko",
  fallbackLng: "ko",
  interpolation: {
    escapeValue: false, // React가 이미 XSS 이스케이프 수행.
  },
  returnNull: false,
});

export default i18n;
