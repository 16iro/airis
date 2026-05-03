// Settings 페이지 — Tabs 3 섹션 (API 키 / 모델 / 언어).
// v0.1 PR 3 기준. 다크 테마·진단·강도 등은 v0.2+에서 추가.

import { useEffect } from "react";
import { ArrowLeft } from "lucide-react";

import { ApiKeyInput } from "@/components/ApiKeyInput";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ANTHROPIC_MODELS } from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";

interface Props {
  onClose: () => void;
}

export function Settings({ onClose }: Props) {
  const settings = useSettingsStore((s) => s.settings);
  const loaded = useSettingsStore((s) => s.loaded);
  const load = useSettingsStore((s) => s.load);
  const update = useSettingsStore((s) => s.update);

  // Settings 페이지 진입 시 백엔드에서 1회 로드 (이미 로드된 경우 noop).
  useEffect(() => {
    if (!loaded) {
      load();
    }
  }, [loaded, load]);

  return (
    <div className="flex min-h-full flex-col bg-background text-foreground">
      <header className="flex h-12 items-center gap-2 border-b border-border px-4">
        <Button variant="ghost" size="sm" onClick={onClose} aria-label="뒤로">
          <ArrowLeft size={18} />
        </Button>
        <h1 className="font-semibold">설정</h1>
      </header>

      <main className="mx-auto w-full max-w-3xl flex-1 px-6 py-8">
        <Tabs defaultValue="api-key" className="w-full">
          <TabsList>
            <TabsTrigger value="api-key">API 키</TabsTrigger>
            <TabsTrigger value="model">모델</TabsTrigger>
            <TabsTrigger value="language">언어</TabsTrigger>
          </TabsList>

          <TabsContent value="api-key">
            <Card>
              <CardHeader>
                <CardTitle>LLM 프로바이더</CardTitle>
                <CardDescription>
                  본인 Anthropic 계정의 API 키를 입력하세요. v0.2부터 OpenAI·로컬 LLM이 추가됩니다.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ApiKeyInput provider="anthropic" label="Anthropic" />
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="model">
            <Card>
              <CardHeader>
                <CardTitle>기본 모델</CardTitle>
                <CardDescription>
                  새 챗 세션에 적용됩니다. 진행 중인 세션엔 영향 X.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {ANTHROPIC_MODELS.map((m) => (
                  <Label
                    key={m.id}
                    className="flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 hover:bg-accent"
                  >
                    <input
                      type="radio"
                      name="model"
                      value={m.id}
                      checked={settings.model === m.id}
                      onChange={() => update({ model: m.id })}
                      className="mt-1"
                    />
                    <span className="flex-1">
                      <span className="block font-medium">{m.id}</span>
                      <span className="block text-sm text-muted-foreground">{m.label}</span>
                    </span>
                  </Label>
                ))}
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="language">
            <Card>
              <CardHeader>
                <CardTitle>UI 언어</CardTitle>
                <CardDescription>
                  v0.1엔 한국어만 지원. 영어는 v0.2 예정.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {[
                  { id: "ko", label: "한국어" },
                  { id: "en", label: "English (v0.2 예정)", disabled: true },
                ].map((opt) => (
                  <Label
                    key={opt.id}
                    className={
                      "flex cursor-pointer items-center gap-3 rounded-md border border-border p-3 hover:bg-accent" +
                      (opt.disabled ? " cursor-not-allowed opacity-50" : "")
                    }
                  >
                    <input
                      type="radio"
                      name="language"
                      value={opt.id}
                      checked={settings.language === opt.id}
                      disabled={opt.disabled}
                      onChange={() => update({ language: opt.id })}
                    />
                    <span>{opt.label}</span>
                  </Label>
                ))}
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>
      </main>
    </div>
  );
}
