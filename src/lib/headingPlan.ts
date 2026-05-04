// Markdown 본문에서 ATX heading을 파일 순서대로 뽑아 path를 부여한다.
// path는 백엔드 paragraphs 테이블의 section_path와 1:1 매칭 — Markdown 파서 slug.rs 미러.
//
// PR 32 (D-070): BookViewer 내부에서 분리. StudySidebar TOC도 같은 함수 사용.

export interface HeadingMeta {
  level: number;
  title: string;
  path: string;
}

/**
 * source에서 ATX heading(`# Title`·`## Title`)을 파일 순서대로 뽑아
 * Markdown 파서(slug.rs) 규칙과 동일한 path를 부여한다.
 *
 * Setext heading(`Title\n===`)은 PR 12 단순화상 미지원 — 거의 안 쓰임.
 */
export function buildHeadingPlan(source: string): HeadingMeta[] {
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
