PDFium 라이브러리는 `pnpm pdfium:setup` 실행 시 이 디렉토리에 다운로드됩니다.
git에는 *이 .keep 파일만* 추적되며, 실제 binary(.so/.dylib/.dll)는 무시됩니다.
Tauri bundle.resources glob 매칭이 빈 디렉토리를 거부하므로 placeholder 역할.
