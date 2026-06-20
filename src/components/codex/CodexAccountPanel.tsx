import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Copy,
  Download,
  ExternalLink,
  FileKey2,
  FileText,
  KeyRound,
  Loader2,
  RefreshCw,
  Save,
  ShieldCheck,
  Trash2,
  UserPlus,
  Zap,
} from "lucide-react";
import { toast } from "sonner";
import {
  codexAccountsApi,
  settingsApi,
  type CodexAccountState,
  type CodexProfileSummary,
  type DeviceAuthLoginResult,
} from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

type ToolStatus = {
  name: string;
  version: string | null;
  latest_version: string | null;
  error: string | null;
  installed_but_broken: boolean;
  env_type: "windows" | "wsl" | "macos" | "linux" | "unknown";
  wsl_distro: string | null;
};

type BusyAction =
  | "refresh"
  | "device"
  | "import"
  | "create"
  | "switch"
  | "delete"
  | "detect"
  | "install"
  | "open";

const defaultForm = {
  name: "OpenAI API Key",
  baseUrl: "https://api.openai.com",
  apiKey: "",
  model: "gpt-5.5",
  reviewModel: "gpt-5.5",
  reasoningEffort: "xhigh",
  notes: "",
};

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 10 || index === 0 ? 0 : 1)} ${units[index]}`;
}

function shortValue(value: string | null | undefined): string {
  if (!value) return "-";
  if (value.length <= 54) return value;
  return `${value.slice(0, 28)}...${value.slice(-18)}`;
}

function profileKindLabel(profile: CodexProfileSummary): string {
  if (profile.codexSystem === "api") return "API Key";
  if (profile.kind === "chat_gpt_login") return "ChatGPT 登录";
  if (profile.kind === "proxy_api_key") return "第三方环境";
  return "自定义";
}

function ProfileRow({
  profile,
  busy,
  onSwitch,
  onDelete,
}: {
  profile: CodexProfileSummary;
  busy: boolean;
  onSwitch: (profile: CodexProfileSummary) => void;
  onDelete: (profile: CodexProfileSummary) => void;
}) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border-default bg-background/70 p-3 md:flex-row md:items-center md:justify-between">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate text-sm font-medium text-foreground">
            {profile.name}
          </span>
          <Badge
            variant={profile.isActive ? "default" : "outline"}
            className={cn(
              "h-5 rounded-md px-1.5 text-[11px]",
              profile.isActive
                ? "bg-emerald-500 text-white hover:bg-emerald-500"
                : "border-border-default text-muted-foreground",
            )}
          >
            {profile.isActive ? "当前" : profileKindLabel(profile)}
          </Badge>
        </div>
        <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
          <span>模型 {profile.model || "-"}</span>
          <span>Base {shortValue(profile.baseUrl)}</span>
          <span>账号 {profile.accountEmail || profile.accountName || "-"}</span>
          <span>配置 {profile.configHash || "-"}</span>
        </div>
        {profile.notes ? (
          <p className="line-clamp-2 text-xs text-muted-foreground">
            {profile.notes}
          </p>
        ) : null}
      </div>
      <div className="flex flex-shrink-0 items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={busy || profile.isActive}
          onClick={() => onSwitch(profile)}
          title="切换到这个 Codex 档案"
        >
          <Zap className="h-3.5 w-3.5" />
          切换
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={() => onDelete(profile)}
          title="删除本地保存的 Codex 档案"
          className="text-red-500 hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/30"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

export function CodexAccountPanel() {
  const [state, setState] = useState<CodexAccountState | null>(null);
  const [toolStatus, setToolStatus] = useState<ToolStatus | null>(null);
  const [deviceAuth, setDeviceAuth] = useState<DeviceAuthLoginResult | null>(
    null,
  );
  const [form, setForm] = useState(defaultForm);
  const [importName, setImportName] = useState("当前 Codex 登录态");
  const [busyAction, setBusyAction] = useState<BusyAction | null>(null);

  const isBusy = busyAction !== null;

  const loadState = useCallback(async (detectTool = true) => {
    const next = await codexAccountsApi.getState();
    setState(next);
    if (detectTool) {
      const tools = await settingsApi.getToolVersions(["codex"]);
      setToolStatus((tools[0] as ToolStatus | undefined) ?? null);
    }
  }, []);

  useEffect(() => {
    setBusyAction("refresh");
    loadState()
      .catch((error) => {
        toast.error("读取 Codex 状态失败", {
          description: error instanceof Error ? error.message : String(error),
        });
      })
      .finally(() => setBusyAction(null));
  }, [loadState]);

  const current = state?.current;
  const activeProfile = useMemo(
    () => state?.profiles.find((profile) => profile.isActive) ?? null,
    [state],
  );

  const refresh = async () => {
    setBusyAction("refresh");
    try {
      await loadState();
      toast.success("Codex 状态已刷新");
    } catch (error) {
      toast.error("刷新失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const detectEnvironment = async () => {
    setBusyAction("detect");
    try {
      const tools = await settingsApi.getToolVersions(["codex"]);
      setToolStatus((tools[0] as ToolStatus | undefined) ?? null);
      toast.success("Codex 环境检测完成");
    } catch (error) {
      toast.error("检测失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const installCodex = async () => {
    setBusyAction("install");
    try {
      await settingsApi.runToolLifecycleAction(["codex"], "install");
      const tools = await settingsApi.getToolVersions(["codex"]);
      setToolStatus((tools[0] as ToolStatus | undefined) ?? null);
      toast.success("Codex 安装/修复命令已执行");
    } catch (error) {
      toast.error("安装/修复失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const startDeviceAuth = async () => {
    setBusyAction("device");
    try {
      const result = await codexAccountsApi.startDeviceAuthLogin();
      setDeviceAuth(result);
      if (result.verificationUrl) {
        await settingsApi.openExternal(result.verificationUrl);
      }
      toast.success("设备码登录已启动", {
        description: result.userCode
          ? `Code: ${result.userCode}`
          : "请按 Codex 输出继续授权",
      });
    } catch (error) {
      toast.error("启动设备码登录失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const importCurrentProfile = async () => {
    setBusyAction("import");
    try {
      const next = await codexAccountsApi.importCurrentProfile({
        name: importName,
        kind: "chat_gpt_login",
      });
      setState(next);
      toast.success("已导入当前 Codex 登录态");
    } catch (error) {
      toast.error("导入失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const createApiProfile = async () => {
    setBusyAction("create");
    try {
      const next = await codexAccountsApi.createProxyProfile({
        name: form.name,
        baseUrl: form.baseUrl,
        apiKey: form.apiKey,
        model: form.model,
        reviewModel: form.reviewModel,
        reasoningEffort: form.reasoningEffort,
        notes: form.notes,
        codexSystem: "api",
      });
      setState(next);
      setForm({ ...defaultForm, apiKey: "" });
      toast.success("已创建 API Key 环境");
    } catch (error) {
      toast.error("创建失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const switchProfile = async (profile: CodexProfileSummary) => {
    setBusyAction("switch");
    try {
      const result = await codexAccountsApi.switchProfile(profile.id);
      setState(result.appState);
      toast.success(result.message);
    } catch (error) {
      toast.error("切换失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const deleteProfile = async (profile: CodexProfileSummary) => {
    setBusyAction("delete");
    try {
      const next = await codexAccountsApi.deleteProfile(profile.id);
      setState(next);
      toast.success(`已删除 ${profile.name}`);
    } catch (error) {
      toast.error("删除失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const copyDeviceCode = async () => {
    if (!deviceAuth?.userCode) return;
    try {
      await copyText(deviceAuth.userCode);
      toast.success("设备码已复制");
    } catch (error) {
      toast.error("复制失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const openCodexFile = async (name: "config.toml" | "auth.json") => {
    setBusyAction("open");
    try {
      await codexAccountsApi.openFile(name);
    } catch (error) {
      toast.error("打开文件失败", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const toolHealthy =
    toolStatus && toolStatus.version && !toolStatus.installed_but_broken;

  return (
    <section className="mb-5 rounded-lg border border-border-default bg-card p-4 shadow-sm">
      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div className="space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-blue-500/10 text-blue-500">
              <KeyRound className="h-4 w-4" />
            </div>
            <h2 className="text-base font-semibold text-foreground">
              Codex 账号与环境
            </h2>
            {activeProfile ? (
              <Badge className="h-5 rounded-md bg-emerald-500 px-1.5 text-[11px] text-white hover:bg-emerald-500">
                {activeProfile.name}
              </Badge>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            打开应用即可管理 Codex 登录态、API Key 环境和本机 Codex 安装状态。
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={isBusy}
            onClick={refresh}
            title="刷新 Codex 账号、配置和环境状态"
          >
            {busyAction === "refresh" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            刷新
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={isBusy}
            onClick={startDeviceAuth}
            title="使用 Codex CLI 官方设备码登录流程"
          >
            {busyAction === "device" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <UserPlus className="h-3.5 w-3.5" />
            )}
            设备码登录
          </Button>
        </div>
      </div>

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        <div className="rounded-lg border border-border-default bg-background/70 p-3">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-foreground">
            <ShieldCheck className="h-4 w-4 text-emerald-500" />
            当前登录
          </div>
          <dl className="space-y-1 text-xs">
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">模式</dt>
              <dd className="truncate text-right text-foreground">
                {current?.authMode || "-"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">账号</dt>
              <dd className="truncate text-right text-foreground">
                {current?.accountEmail || current?.accountName || "-"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">模型</dt>
              <dd className="truncate text-right text-foreground">
                {current?.model || "-"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">会话</dt>
              <dd className="truncate text-right text-foreground">
                {formatBytes(current?.sessionSize ?? 0)}
              </dd>
            </div>
          </dl>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={isBusy || !current?.configExists}
              onClick={() => openCodexFile("config.toml")}
              title="打开 Codex config.toml"
            >
              <FileText className="h-3.5 w-3.5" />
              配置
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={isBusy || !current?.authExists}
              onClick={() => openCodexFile("auth.json")}
              title="打开 Codex auth.json"
            >
              <FileKey2 className="h-3.5 w-3.5" />
              授权
            </Button>
          </div>
        </div>

        <div className="rounded-lg border border-border-default bg-background/70 p-3">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-foreground">
            <Download className="h-4 w-4 text-blue-500" />
            本机环境
          </div>
          <dl className="space-y-1 text-xs">
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">状态</dt>
              <dd
                className={cn(
                  "truncate text-right",
                  toolHealthy ? "text-emerald-600" : "text-amber-600",
                )}
              >
                {toolHealthy
                  ? "已安装"
                  : toolStatus?.installed_but_broken
                    ? "已安装但不可运行"
                    : "未就绪"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">版本</dt>
              <dd className="truncate text-right text-foreground">
                {toolStatus?.version || "-"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">环境</dt>
              <dd className="truncate text-right text-foreground">
                {toolStatus?.env_type || "-"}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">错误</dt>
              <dd className="truncate text-right text-foreground">
                {shortValue(toolStatus?.error)}
              </dd>
            </div>
          </dl>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={isBusy}
              onClick={detectEnvironment}
              title="检测 Codex CLI 是否可用"
            >
              {busyAction === "detect" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
              检测
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={isBusy}
              onClick={installCodex}
              title="运行现有 Codex 安装/修复流程"
            >
              {busyAction === "install" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="h-3.5 w-3.5" />
              )}
              安装/修复
            </Button>
          </div>
        </div>

        <div className="rounded-lg border border-border-default bg-background/70 p-3">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-foreground">
            <Save className="h-4 w-4 text-violet-500" />
            保存当前登录态
          </div>
          <div className="flex gap-2">
            <Input
              value={importName}
              onChange={(event) => setImportName(event.target.value)}
              placeholder="档案名称"
              className="h-8 text-xs"
            />
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={isBusy || !current?.authExists}
              onClick={importCurrentProfile}
              title="把当前 Codex config.toml/auth.json 保存为可切换档案"
            >
              {busyAction === "import" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Save className="h-3.5 w-3.5" />
              )}
              导入
            </Button>
          </div>
          <p className="mt-2 text-xs text-muted-foreground">
            设备码授权完成后刷新，再导入即可把官方账号保存成可切换 profile。
          </p>
        </div>
      </div>

      {deviceAuth ? (
        <div className="mt-3 rounded-lg border border-blue-200 bg-blue-50 p-3 text-xs text-blue-950 dark:border-blue-900/60 dark:bg-blue-950/30 dark:text-blue-100">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <span className="font-medium">设备码登录进行中</span>
            <div className="flex gap-2">
              {deviceAuth.userCode ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={copyDeviceCode}
                  title="复制设备码"
                  className="h-7 border-blue-300 bg-white/80 text-blue-700 hover:bg-blue-100 dark:border-blue-800 dark:bg-blue-950/50 dark:text-blue-100"
                >
                  <Copy className="h-3.5 w-3.5" />
                  {deviceAuth.userCode}
                </Button>
              ) : null}
              {deviceAuth.verificationUrl ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    settingsApi.openExternal(deviceAuth.verificationUrl!)
                  }
                  title="打开设备码登录页面"
                  className="h-7 border-blue-300 bg-white/80 text-blue-700 hover:bg-blue-100 dark:border-blue-800 dark:bg-blue-950/50 dark:text-blue-100"
                >
                  <ExternalLink className="h-3.5 w-3.5" />
                  打开
                </Button>
              ) : null}
            </div>
          </div>
          <p className="break-all text-blue-900/80 dark:text-blue-100/80">
            {deviceAuth.verificationUrl || deviceAuth.message}
          </p>
        </div>
      ) : null}

      <div className="mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <h3 className="text-sm font-medium text-foreground">
              Codex profiles
            </h3>
            <span className="text-xs text-muted-foreground">
              {state?.profiles.length ?? 0} 个
            </span>
          </div>
          {state?.profiles.length ? (
            <div className="space-y-2">
              {state.profiles.map((profile) => (
                <ProfileRow
                  key={profile.id}
                  profile={profile}
                  busy={isBusy}
                  onSwitch={switchProfile}
                  onDelete={deleteProfile}
                />
              ))}
            </div>
          ) : (
            <div className="rounded-lg border border-dashed border-border-default bg-background/50 p-4 text-sm text-muted-foreground">
              还没有保存的 Codex profile。先设备码登录或创建 API Key 环境。
            </div>
          )}
        </div>

        <div className="rounded-lg border border-border-default bg-background/70 p-3">
          <div className="mb-3 flex items-center gap-2 text-sm font-medium text-foreground">
            <FileKey2 className="h-4 w-4 text-amber-500" />
            创建 API Key 环境
          </div>
          <div className="space-y-2">
            <Input
              value={form.name}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, name: event.target.value }))
              }
              placeholder="环境名称"
              className="h-8 text-xs"
            />
            <Input
              value={form.baseUrl}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, baseUrl: event.target.value }))
              }
              placeholder="Base URL"
              className="h-8 text-xs"
            />
            <Input
              type="password"
              value={form.apiKey}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, apiKey: event.target.value }))
              }
              placeholder="API Key"
              className="h-8 text-xs"
            />
            <div className="grid grid-cols-2 gap-2">
              <Input
                value={form.model}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, model: event.target.value }))
                }
                placeholder="模型"
                className="h-8 text-xs"
              />
              <Input
                value={form.reviewModel}
                onChange={(event) =>
                  setForm((prev) => ({
                    ...prev,
                    reviewModel: event.target.value,
                  }))
                }
                placeholder="Review 模型"
                className="h-8 text-xs"
              />
            </div>
            <select
              value={form.reasoningEffort}
              onChange={(event) =>
                setForm((prev) => ({
                  ...prev,
                  reasoningEffort: event.target.value,
                }))
              }
              className="h-8 w-full rounded-md border border-border-default bg-background px-3 text-xs text-foreground focus:outline-none focus:ring-2 focus:ring-blue-500/20"
            >
              <option value="minimal">minimal</option>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
              <option value="xhigh">xhigh</option>
            </select>
            <Input
              value={form.notes}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, notes: event.target.value }))
              }
              placeholder="备注"
              className="h-8 text-xs"
            />
            <Button
              type="button"
              size="sm"
              className="w-full"
              disabled={isBusy}
              onClick={createApiProfile}
              title="保存一个 API Key profile，并生成 Codex API 模式配置"
            >
              {busyAction === "create" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Save className="h-3.5 w-3.5" />
              )}
              创建环境
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
