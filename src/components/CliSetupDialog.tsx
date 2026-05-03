// PR 24 (D-066) — CLI 연결 다이얼로그.
//
// 흐름: 런타임 감지 → CLI 설치 상태 → (필요시) 설치 → 로그인 상태 → (필요시) 로그인.
// 각 단계는 명령 호출이 끝나야 다음으로 넘어감 — 진행률 이벤트는 없으므로 *대기 UI*로 처리.
//
// PR 24 시점엔 Anthropic만 완전 지원. Gemini/Codex는 PR 25/26에서 동일 패턴으로 채움.

import { CheckCircle2, ExternalLink, Loader2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError } from "@/lib/types";
import type {
  ClaudeAuthInfo,
  CliLoginOutcome,
  CliStatus,
  GeminiAuthInfo,
  Provider,
  RuntimeInfo,
} from "@/lib/types";

interface AuthState {
  logged_in: boolean;
  /** anthropic 전용 — 구독·이메일 등 추가 정보. */
  detail?: ClaudeAuthInfo;
}

interface Props {
  provider: Provider;
  onClose: () => void;
  /** 로그인까지 완료된 경우 호출. Settings에서 auth_mode를 cli로 전환할 때 후속 처리에 사용. */
  onComplete?: () => void;
}

type StepState =
  | { kind: "loading" }
  | { kind: "ok" }
  | { kind: "error"; message: string };

export function CliSetupDialog({ provider, onClose, onComplete }: Props) {
  const { t } = useTranslation();
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [runtimeStep, setRuntimeStep] = useState<StepState>({ kind: "loading" });
  const [cli, setCli] = useState<CliStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [authInfo, setAuthInfo] = useState<AuthState | null>(null);
  const [loginRunning, setLoginRunning] = useState(false);
  const [terminalInstruction, setTerminalInstruction] = useState<{
    command: string;
    hint: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const completedRef = useRef(false);

  // 1) 런타임 감지.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const info = await api.cliRuntimeDetect();
        if (cancelled) return;
        setRuntime(info);
        setRuntimeStep({ kind: "ok" });
      } catch (e) {
        if (cancelled) return;
        setRuntimeStep({
          kind: "error",
          message: isAppError(e) ? appErrorMessage(e) : String(e),
        });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // 2) 런타임 확인되면 CLI 상태 점검.
  useEffect(() => {
    if (runtimeStep.kind !== "ok") return;
    let cancelled = false;
    void (async () => {
      try {
        const status = await api.cliStatus(provider);
        if (!cancelled) setCli(status);
      } catch (e) {
        if (!cancelled) setError(isAppError(e) ? appErrorMessage(e) : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [runtimeStep.kind, provider]);

  // 3) CLI 설치돼 있으면 인증 상태 확인. 프로바이더별 다른 명령 사용.
  useEffect(() => {
    if (!cli || !cli.installed) return;
    let cancelled = false;
    void (async () => {
      try {
        if (provider === "anthropic") {
          const info = await api.cliAuthStatusClaude();
          if (!cancelled) {
            setAuthInfo({ logged_in: info.logged_in, detail: info });
          }
        } else if (provider === "gemini") {
          const info: GeminiAuthInfo = await api.cliAuthStatusGemini();
          if (!cancelled) setAuthInfo({ logged_in: info.logged_in });
        } else if (provider === "openai") {
          const info = await api.cliAuthStatusCodex();
          if (!cancelled) setAuthInfo({ logged_in: info.logged_in });
        }
      } catch (e) {
        if (!cancelled) setError(isAppError(e) ? appErrorMessage(e) : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [cli, provider]);

  // 4) 모든 단계 통과하면 onComplete 1회 발화.
  useEffect(() => {
    if (completedRef.current) return;
    if (runtimeStep.kind !== "ok") return;
    if (!cli?.installed) return;
    if (!authInfo?.logged_in) return;
    completedRef.current = true;
    onComplete?.();
  }, [runtimeStep.kind, cli, authInfo, onComplete]);

  async function install(forceLatest: boolean) {
    setInstalling(true);
    setError(null);
    try {
      const next = await api.cliInstallProvider(provider, forceLatest);
      setCli(next);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setInstalling(false);
    }
  }

  async function login(useConsole: boolean) {
    setLoginRunning(true);
    setError(null);
    setTerminalInstruction(null);
    try {
      const outcome: CliLoginOutcome = await api.cliLogin(provider, useConsole);
      if (outcome.kind === "TerminalInstruction") {
        setTerminalInstruction({ command: outcome.command, hint: outcome.hint });
      }
      // 로그인 후 인증 상태 다시 확인.
      if (provider === "anthropic") {
        const info = await api.cliAuthStatusClaude();
        setAuthInfo({ logged_in: info.logged_in, detail: info });
      } else if (provider === "gemini") {
        const info = await api.cliAuthStatusGemini();
        setAuthInfo({ logged_in: info.logged_in });
      } else if (provider === "openai") {
        const info = await api.cliAuthStatusCodex();
        setAuthInfo({ logged_in: info.logged_in });
      }
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setLoginRunning(false);
    }
  }

  async function recheckAuth() {
    setError(null);
    try {
      if (provider === "anthropic") {
        const info = await api.cliAuthStatusClaude();
        setAuthInfo({ logged_in: info.logged_in, detail: info });
      } else if (provider === "gemini") {
        const info = await api.cliAuthStatusGemini();
        setAuthInfo({ logged_in: info.logged_in });
      } else if (provider === "openai") {
        const info = await api.cliAuthStatusCodex();
        setAuthInfo({ logged_in: info.logged_in });
      }
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="cli-setup-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <Card className="w-full max-w-lg" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <CardTitle id="cli-setup-title">{t("cli_setup.dialog_title")}</CardTitle>
            <Button variant="ghost" size="sm" className="h-7 px-2" onClick={onClose}>
              <X size={14} />
            </Button>
          </div>
          <p className="text-sm text-muted-foreground">{t("cli_setup.dialog_desc")}</p>
        </CardHeader>
        <CardContent className="space-y-4 text-sm">
          <RuntimeStep runtime={runtime} step={runtimeStep} />
          {runtimeStep.kind === "ok" ? (
            <CliStep
              cli={cli}
              provider={provider}
              installing={installing}
              onInstall={install}
            />
          ) : null}
          {cli?.installed ? (
            <AuthStep
              provider={provider}
              authInfo={authInfo}
              loginRunning={loginRunning}
              terminalInstruction={terminalInstruction}
              onLogin={login}
              onRecheck={recheckAuth}
            />
          ) : null}
          {error ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive">
              <p className="font-medium">{t("cli_setup.error_title")}</p>
              <p>{error}</p>
            </div>
          ) : null}
          <div className="flex justify-end pt-1">
            <Button variant="outline" size="sm" onClick={onClose}>
              {t("cli_setup.close")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function RuntimeStep({
  runtime,
  step,
}: {
  runtime: RuntimeInfo | null;
  step: StepState;
}) {
  const { t } = useTranslation();

  if (step.kind === "loading") {
    return (
      <Row icon={<Loader2 className="animate-spin" size={14} />} label={t("cli_setup.step_runtime")} />
    );
  }
  if (step.kind === "error") {
    return (
      <div className="space-y-1">
        <Row label={t("cli_setup.node_missing_title")} />
        <p className="pl-5 text-xs text-muted-foreground">
          {t("cli_setup.node_missing_desc")}
        </p>
        <button
          type="button"
          className="ml-5 inline-flex items-center gap-1 text-xs text-primary underline"
          onClick={() =>
            void import("@tauri-apps/plugin-opener").then(({ openUrl }) =>
              openUrl("https://nodejs.org"),
            )
          }
        >
          <ExternalLink size={11} />
          {t("cli_setup.node_link")}
        </button>
      </div>
    );
  }
  return (
    <Row
      icon={<CheckCircle2 size={14} className="text-emerald-600" />}
      label={t("cli_setup.node_detected", {
        node: runtime?.node_version ?? "?",
        npm: runtime?.npm_version ?? "?",
      })}
    />
  );
}

function CliStep({
  cli,
  provider,
  installing,
  onInstall,
}: {
  cli: CliStatus | null;
  provider: Provider;
  installing: boolean;
  onInstall: (forceLatest: boolean) => void;
}) {
  const { t } = useTranslation();
  if (!cli) {
    return <Row icon={<Loader2 className="animate-spin" size={14} />} label="…" />;
  }
  if (!cli.installed) {
    return (
      <div className="space-y-2">
        <Row
          label={t("cli_setup.cli_not_installed", { provider: providerLabel(provider) })}
        />
        <Button
          size="sm"
          variant="default"
          disabled={installing}
          onClick={() => onInstall(false)}
          className="ml-5"
        >
          {installing ? t("cli_setup.installing") : t("cli_setup.install_now")}
        </Button>
      </div>
    );
  }
  return (
    <div className="space-y-1">
      <Row
        icon={<CheckCircle2 size={14} className="text-emerald-600" />}
        label={t("cli_setup.cli_installed", {
          provider: providerLabel(provider),
          version: cli.version ?? "?",
        })}
      />
      <Button
        size="sm"
        variant="ghost"
        disabled={installing}
        onClick={() => onInstall(true)}
        className="ml-5 h-7 px-2 text-xs"
      >
        {installing ? t("cli_setup.installing") : t("cli_setup.update_now")}
      </Button>
    </div>
  );
}

function AuthStep({
  provider,
  authInfo,
  loginRunning,
  terminalInstruction,
  onLogin,
  onRecheck,
}: {
  provider: Provider;
  authInfo: AuthState | null;
  loginRunning: boolean;
  terminalInstruction: { command: string; hint: string } | null;
  onLogin: (useConsole: boolean) => void;
  onRecheck: () => void;
}) {
  const { t } = useTranslation();
  if (!authInfo) {
    return (
      <Row
        icon={<Loader2 className="animate-spin" size={14} />}
        label={t("cli_setup.step_login")}
      />
    );
  }
  if (authInfo.logged_in) {
    const detail = authInfo.detail;
    const isConsole = detail?.auth_method === "console";
    return (
      <Row
        icon={<CheckCircle2 size={14} className="text-emerald-600" />}
        label={
          isConsole
            ? t("cli_setup.auth_logged_in_console")
            : detail
              ? t("cli_setup.auth_logged_in", {
                  auth: detail.email ?? detail.auth_method ?? "?",
                  plan: detail.subscription_type ?? "?",
                })
              : t("cli_setup.auth_logged_in_simple")
        }
      />
    );
  }
  return (
    <div className="space-y-2">
      <Row label={t("cli_setup.auth_not_logged_in")} />
      {provider === "anthropic" ? (
        <div className="ml-5 flex flex-wrap gap-2">
          <Button size="sm" disabled={loginRunning} onClick={() => onLogin(false)}>
            {loginRunning ? t("cli_setup.logging_in") : t("cli_setup.login_subscription")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={loginRunning}
            onClick={() => onLogin(true)}
          >
            {t("cli_setup.login_console")}
          </Button>
        </div>
      ) : (
        <div className="ml-5 space-y-2">
          <Button size="sm" disabled={loginRunning} onClick={() => onLogin(false)}>
            {loginRunning ? t("cli_setup.logging_in") : t("cli_setup.login_show_command")}
          </Button>
          {terminalInstruction ? (
            <div className="rounded-md border border-border bg-muted p-2 text-xs">
              <code className="block font-mono">{terminalInstruction.command}</code>
              <p className="mt-1 text-muted-foreground">{terminalInstruction.hint}</p>
              <Button
                size="sm"
                variant="outline"
                className="mt-2 h-6 px-2 text-[11px]"
                onClick={onRecheck}
              >
                {t("cli_setup.recheck")}
              </Button>
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}

function Row({ icon, label }: { icon?: React.ReactNode; label: string }) {
  return (
    <div className="flex items-center gap-2">
      {icon ?? <span className="size-3 rounded-full border border-muted-foreground/40" />}
      <span>{label}</span>
    </div>
  );
}

function providerLabel(provider: Provider): string {
  const labels: Record<Provider, string> = {
    anthropic: "Claude Code",
    gemini: "Gemini CLI",
    openai: "Codex CLI",
  };
  return labels[provider];
}
