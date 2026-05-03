// 파일 뷰어 — 단일 .md / .txt 본문 표시.
// v0.1: react-markdown + remark-gfm. 코드 syntax highlighting은 v0.3.

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FileText, FolderOpen, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { Button } from "@/components/ui/button";
import { useFileStore } from "@/store/fileStore";

export function FileViewer() {
  const { t } = useTranslation();
  const meta = useFileStore((s) => s.meta);
  const content = useFileStore((s) => s.content);
  const open = useFileStore((s) => s.open);
  const close = useFileStore((s) => s.close);
  const error = useFileStore((s) => s.error);

  async function handleOpen() {
    const path = await openDialog({
      multiple: false,
      directory: false,
      filters: [
        {
          name: "Text/Markdown",
          extensions: ["md", "markdown", "txt"],
        },
      ],
    });
    if (typeof path === "string") {
      try {
        await open(path);
      } catch {
        // 에러는 store가 보관 — 화면에 노출.
      }
    }
  }

  if (!meta || !content) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
        <FileText size={48} className="text-muted-foreground" />
        <h2 className="text-lg font-medium">{t("workspace.no_file_title")}</h2>
        <p className="max-w-sm text-sm text-muted-foreground">
          {t("workspace.no_file_hint")}
        </p>
        <Button onClick={handleOpen}>
          <FolderOpen size={16} />
          {t("workspace.open_file_button")}
        </Button>
        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border bg-muted/30 px-4 py-2">
        <span className="truncate text-sm font-medium" title={meta.path}>
          {t("workspace.file_meta", {
            name: meta.name,
            count: meta.char_count.toLocaleString(),
          })}
        </span>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="sm"
            onClick={handleOpen}
            aria-label={t("topbar.open_file")}
            title={t("topbar.open_file")}
          >
            <FolderOpen size={16} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void close()}
            aria-label={t("topbar.close_file")}
            title={t("topbar.close_file")}
          >
            <X size={16} />
          </Button>
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-6 py-4">
        <article className="markdown-body max-w-3xl">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
        </article>
      </div>
    </div>
  );
}
