import React from "react";
import ReactDOM from "react-dom/client";

// 폰트 — Pretendard(본문, 한국어 우선) + Geist Mono(코드).
// design/ui/colors-theme.md "폰트" 절 참조.
import "pretendard/dist/web/variable/pretendardvariable.css";
import "@fontsource-variable/geist-mono";

import "@/styles/tokens.css";
import App from "@/App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
