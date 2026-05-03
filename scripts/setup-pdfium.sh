#!/usr/bin/env bash
# scripts/setup-pdfium.sh — PDFium 라이브러리를 src-tauri/resources/pdfium/ 에 다운로드.
#
# 출처: https://github.com/bblanchon/pdfium-binaries (MIT, prebuilt PDFium binaries).
# 저장소가 chromium/{revision} tag로 release를 publish하며 OS·arch별 tgz를 첨부.
#
# 사용:
#   pnpm pdfium:setup            # 또는 직접 실행
#   PDFIUM_VERSION=6996 pnpm pdfium:setup
#
# 결과:
#   src-tauri/resources/pdfium/
#   ├── LICENSE
#   ├── VERSION (PDFium revision 기록)
#   ├── include/   (헤더 — 사용 X, 압축에 포함되어 같이 풀림)
#   └── lib/       (libpdfium.so / .dylib / pdfium.dll — 런타임 로드 대상)

set -euo pipefail

PDFIUM_VERSION="${PDFIUM_VERSION:-6996}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESOURCE_DIR="$PROJECT_ROOT/src-tauri/resources/pdfium"

# OS·아키텍처 감지.
case "$(uname -s)" in
  Linux*) OS="linux" ;;
  Darwin*) OS="mac" ;;
  *)
    echo "지원하지 않는 OS: $(uname -s). Windows는 scripts/setup-pdfium.ps1 참조." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) ARCH="x64" ;;
  aarch64 | arm64) ARCH="arm64" ;;
  *)
    echo "지원하지 않는 아키텍처: $(uname -m)" >&2
    exit 1
    ;;
esac

ASSET="pdfium-${OS}-${ARCH}.tgz"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F${PDFIUM_VERSION}/${ASSET}"

echo "→ PDFium ${PDFIUM_VERSION} (${OS}-${ARCH}) 다운로드"
echo "  URL: $URL"

mkdir -p "$RESOURCE_DIR"
TMP_TGZ="$(mktemp)"
trap 'rm -f "$TMP_TGZ"' EXIT

# curl: -L follow redirects, -f fail on HTTP error, -sS quiet but show errors, --retry 3.
curl -fL -sS --retry 3 -o "$TMP_TGZ" "$URL"

# 기존 binary 정리 후 새로 풀기. PLACEHOLDER 파일은 풀기 후 다시 박는다 (.gitignore 추적 보존).
rm -rf "$RESOURCE_DIR"/{lib,include,LICENSE,VERSION}
tar -xzf "$TMP_TGZ" -C "$RESOURCE_DIR"

# VERSION 마커 — runtime stale check 용도.
echo "$PDFIUM_VERSION" > "$RESOURCE_DIR/VERSION"

# Tauri bundle.resources glob이 *항상 매칭*되도록 PLACEHOLDER 보존.
mkdir -p "$RESOURCE_DIR/lib" "$RESOURCE_DIR/include"
echo "do not delete — keeps glob matching" > "$RESOURCE_DIR/lib/PLACEHOLDER"
echo "do not delete — keeps glob matching" > "$RESOURCE_DIR/include/PLACEHOLDER"

LIB_PATH="$RESOURCE_DIR/lib"
if [ ! -d "$LIB_PATH" ]; then
  echo "오류: lib/ 디렉토리가 압축에 없습니다. release asset 구조 확인 필요." >&2
  exit 1
fi

echo "✓ PDFium 설치 완료: $LIB_PATH"
ls -la "$LIB_PATH"
