// MD/HTML 책 뷰어 — 헤딩 클릭 시 활성 섹션 박힘.
//
// 동작:
//   * MD: ReactMarkdown으로 렌더 + 커스텀 h1~h6 컴포넌트가 클릭·앵커 처리
//   * HTML: 백엔드는 sanitize 결과만 반환할 거지만, 추가 안전을 위해 *iframe sandbox*에 띄움
//   * 활성 섹션 = 사용자가 클릭한 헤딩의 path (Markdown 파서 슬러그 규칙과 동일)
//   * 검색 결과/인용 클릭 시 pendingScrollPath로 들어오면 자동 스크롤

import { convertFileSrc } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Loader2, X } from "lucide-react";
import * as pdfjsLib from "pdfjs-dist";
import workerSrc from "pdfjs-dist/build/pdf.worker.mjs?url";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useActiveBookStore } from "@/store/activeBookStore";

// pdfjs worker — Vite + ?url import 패턴. 한 번만 등록.
if (typeof window !== "undefined") {
  pdfjsLib.GlobalWorkerOptions.workerSrc = workerSrc;
}

interface HeadingMeta {
  level: number;
  title: string;
  path: string;
}

export function BookViewer() {
  const { t } = useTranslation();
  const content = useActiveBookStore((s) => s.content);
  const loading = useActiveBookStore((s) => s.loading);
  const sectionPath = useActiveBookStore((s) => s.sectionPath);
  const setSection = useActiveBookStore((s) => s.setSection);
  const close = useActiveBookStore((s) => s.close);
  const consumePendingScroll = useActiveBookStore((s) => s.consumePendingScroll);

  const containerRef = useRef<HTMLDivElement>(null);
  const headingRefs = useRef<Map<string, HTMLElement>>(new Map());

  // 검색 결과·인용 클릭 시 pendingScrollPath로 들어온 path로 스크롤.
  useEffect(() => {
    if (!content) return;
    const path = consumePendingScroll();
    if (!path) return;
    // 다음 paint에서 헤딩 등록 후 스크롤.
    requestAnimationFrame(() => {
      const el = headingRefs.current.get(path);
      el?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  }, [content, consumePendingScroll]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        <Loader2 className="animate-spin" size={20} />
      </div>
    );
  }
  if (!content) {
    return null;
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border px-4 py-2">
        <span className="truncate text-xs text-muted-foreground">
          {sectionPath ? sectionPath.replace(/\//g, " ") : t("bookviewer.no_active_section")}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2"
          onClick={() => void close()}
          aria-label={t("bookviewer.close")}
        >
          <X size={14} />
        </Button>
      </div>
      <div ref={containerRef} className="flex-1 overflow-y-auto px-6 py-4">
        {content.format === "pdf" ? (
          <PdfContent sourcePath={content.source_path} />
        ) : content.format === "html" ? (
          <HtmlContent html={content.content} />
        ) : (
          <MarkdownContent
            source={content.content}
            activeSectionPath={sectionPath}
            registerHeading={(path, el) => {
              if (el) headingRefs.current.set(path, el);
              else headingRefs.current.delete(path);
            }}
            onHeadingClick={(path) => void setSection(path)}
          />
        )}
      </div>
    </div>
  );
}

function PdfContent({ sourcePath }: { sourcePath: string }) {
  const consumePendingPage = useActiveBookStore((s) => s.consumePendingPage);
  const [doc, setDoc] = useState<pdfjsLib.PDFDocumentProxy | null>(null);
  const [totalPages, setTotalPages] = useState(0);
  const [pageNum, setPageNum] = useState(1);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // PDF 로드 — convertFileSrc로 asset:// URL 생성. then callback에서 pendingPage도 같이 적용.
  useEffect(() => {
    let cancelled = false;
    const initialLoading = (() => {
      setLoading(true);
      setError(null);
      return true;
    })();
    void initialLoading;
    const url = convertFileSrc(sourcePath);
    const task = pdfjsLib.getDocument({ url });
    task.promise
      .then((d) => {
        if (cancelled) {
          void d.destroy();
          return;
        }
        const target = consumePendingPage();
        const start = target && target >= 1 && target <= d.numPages ? target : 1;
        setDoc(d);
        setTotalPages(d.numPages);
        setPageNum(start);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        console.error("PDF load failed:", e);
        setError(e instanceof Error ? e.message : String(e));
        setLoading(false);
      });
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [sourcePath, consumePendingPage]);

  // 현재 페이지 렌더.
  useEffect(() => {
    if (!doc || !canvasRef.current) return;
    let cancelled = false;
    void doc.getPage(pageNum).then((page) => {
      if (cancelled || !canvasRef.current) return;
      const canvas = canvasRef.current;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const viewport = page.getViewport({ scale: 1.5 });
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      const renderTask = page.render({ canvasContext: ctx, viewport, canvas });
      renderTask.promise.catch((e: unknown) => {
        if (!cancelled) console.error("PDF page render failed:", e);
      });
    });
    return () => {
      cancelled = true;
    };
  }, [doc, pageNum]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="animate-spin" size={20} />
      </div>
    );
  }
  if (error || !doc) {
    return (
      <div className="text-sm text-destructive" role="alert">
        PDF 로드 실패: {error ?? "알 수 없는 오류"}
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center gap-4">
      <div className="sticky top-0 z-10 flex items-center gap-2 rounded-md bg-card/90 px-2 py-1 text-xs backdrop-blur">
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2"
          onClick={() => setPageNum((p) => Math.max(1, p - 1))}
          disabled={pageNum <= 1}
          aria-label="이전 페이지"
        >
          <ChevronLeft size={14} />
        </Button>
        <Input
          type="number"
          min={1}
          max={totalPages}
          value={pageNum}
          onChange={(e) => {
            const n = parseInt(e.target.value, 10);
            if (!Number.isNaN(n) && n >= 1 && n <= totalPages) setPageNum(n);
          }}
          className="h-7 w-16 text-center"
          aria-label="페이지 번호"
        />
        <span className="text-muted-foreground">/ {totalPages}</span>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2"
          onClick={() => setPageNum((p) => Math.min(totalPages, p + 1))}
          disabled={pageNum >= totalPages}
          aria-label="다음 페이지"
        >
          <ChevronRight size={14} />
        </Button>
      </div>
      <canvas ref={canvasRef} className="max-w-full bg-white shadow-md" />
    </div>
  );
}

function HtmlContent({ html }: { html: string }) {
  // sanitize는 백엔드에서 ammonia로 이미 처리. 추가 안전을 위해 sandbox iframe.
  // srcDoc 사용 — 외부 src 로딩 X.
  return (
    <iframe
      title="book-html"
      sandbox=""
      className="h-full w-full border-0"
      srcDoc={html}
    />
  );
}

interface MarkdownContentProps {
  source: string;
  activeSectionPath: string | null;
  registerHeading: (path: string, el: HTMLElement | null) => void;
  onHeadingClick: (path: string) => void;
}

function MarkdownContent({
  source,
  activeSectionPath,
  registerHeading,
  onHeadingClick,
}: MarkdownContentProps) {
  // 매 render마다 *새 카운터*로 components 생성 — ReactMarkdown이 첫 헤딩부터 순서대로 호출.
  // useMemo 사용 X (cache되면 두 번째 render에서 카운터 누적 — 잘못된 path 부여).
  const traversal = useMemo(() => buildHeadingPlan(source), [source]);
  const counter = { idx: 0 };
  const take = (): HeadingMeta | null => traversal[counter.idx++] ?? null;
  const components = makeHeadingComponents(
    take,
    activeSectionPath,
    registerHeading,
    onHeadingClick,
  );

  return (
    <div className="markdown-body">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {source}
      </ReactMarkdown>
    </div>
  );
}

// ---- heading plan: ATX heading만 ATX 라인 순서로 path 계산 ----------------

const SLUG_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

/**
 * source에서 ATX heading(`# Title`·`## Title`)을 파일 순서대로 뽑아
 * Markdown 파서(slug.rs) 규칙과 동일한 path를 부여한다.
 *
 * Setext heading(`Title\n===`)은 PR 12 단순화상 미지원 — 거의 안 쓰임.
 */
function buildHeadingPlan(source: string): HeadingMeta[] {
  const out: HeadingMeta[] = [];
  const lines = source.split("\n");
  const used = new Set<string>();
  let chapterPath: string | null = null;
  let chapterCounter = 0;
  const hasAnyH1 = lines.some((l) => /^#\s+\S/.test(l));
  const chapterThreshold = hasAnyH1 ? 1 : 2;

  for (const line of lines) {
    const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (!match) continue;
    const level = match[1].length;
    const title = match[2].trim();
    if (!title) continue;

    let path: string;
    if (level <= chapterThreshold) {
      chapterCounter += 1;
      const n = parseChapterNumber(title) ?? chapterCounter;
      path = dedupe(`Ch${String(n).padStart(2, "0")}`, used);
      chapterPath = path;
    } else {
      const token = sectionToken(title);
      const prefixed = chapterPath ? `${chapterPath}/${token}` : token;
      path = dedupe(prefixed, used);
    }
    used.add(path);
    out.push({ level, title, path });
  }
  return out;
}

function makeHeadingComponents(
  take: () => HeadingMeta | null,
  activePath: string | null,
  registerHeading: (path: string, el: HTMLElement | null) => void,
  onClick: (path: string) => void,
): Components {
  function build(level: number) {
    const tag = `h${level}` as keyof React.JSX.IntrinsicElements;
    return function HeadingComponent({
      children,
      ...rest
    }: React.HTMLAttributes<HTMLHeadingElement>) {
      const meta = take();
      if (!meta) {
        return React.createElement(tag, rest, children);
      }
      const isActive = meta.path === activePath;
      const className = cn(
        "cursor-pointer transition-colors hover:text-primary",
        isActive && "text-primary",
        (rest as { className?: string }).className,
      );
      return React.createElement(
        tag,
        {
          ...rest,
          ref: (el: HTMLHeadingElement | null) => registerHeading(meta.path, el),
          onClick: () => onClick(meta.path),
          className,
          title: meta.path,
        },
        children,
      );
    };
  }
  return {
    h1: build(1),
    h2: build(2),
    h3: build(3),
    h4: build(4),
    h5: build(5),
    h6: build(6),
  };
}

// ---- slug 규칙 (Markdown 파서 slug.rs 미러) -------------------------------

function parseChapterNumber(title: string): number | null {
  const lower = title.toLowerCase().trim();
  for (const prefix of ["chapter ", "ch. ", "ch.", "ch "]) {
    if (lower.startsWith(prefix)) {
      const n = leadingDigits(lower.slice(prefix.length).trimStart());
      if (n != null) return n;
    }
  }
  if (lower.startsWith("ch")) {
    const n = leadingDigits(lower.slice(2));
    if (n != null) return n;
  }
  if (title.startsWith("제")) {
    const n = leadingDigits(title.slice(1).trimStart());
    if (n != null) return n;
  }
  const n = leadingDigits(title);
  if (n != null) {
    const after = title.slice(String(n).length).trimStart();
    if (after.length === 0 || after.startsWith("장")) return n;
  }
  return null;
}

function leadingDigits(s: string): number | null {
  const m = /^(\d+)/.exec(s);
  return m ? parseInt(m[1], 10) : null;
}

function sectionToken(title: string): string {
  let out = "";
  let prevDash = false;
  for (const ch of title.trim()) {
    if (isAlphanumOrCjk(ch)) {
      out += ch;
      prevDash = false;
    } else if (!prevDash && out.length > 0) {
      out += "-";
      prevDash = true;
    }
  }
  while (out.endsWith("-")) out = out.slice(0, -1);
  return out.length === 0 ? "§untitled" : `§${out}`;
}

function isAlphanumOrCjk(c: string): boolean {
  if (/[a-zA-Z0-9]/.test(c)) return true;
  const code = c.codePointAt(0) ?? 0;
  // 한글 음절 / 한자 / 가나
  return (
    (code >= 0xac00 && code <= 0xd7a3) ||
    (code >= 0x4e00 && code <= 0x9fff) ||
    (code >= 0x3040 && code <= 0x30ff)
  );
}

function dedupe(base: string, used: Set<string>): string {
  if (!used.has(base)) return base;
  let n = 2;
  while (used.has(`${base}-${n}`)) n++;
  return `${base}-${n}`;
}

// SLUG_RE는 다른 곳에서 사용될 가능성을 위해 export 안 함 — 이 모듈 내부 검증용.
void SLUG_RE;
