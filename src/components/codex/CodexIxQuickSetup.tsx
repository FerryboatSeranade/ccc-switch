import { useCallback, useEffect, useState } from "react";
import { Eye, EyeOff, KeyRound, Loader2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { codexGogoaisApi } from "@/lib/api/codexGogoais";
import { providersApi } from "@/lib/api/providers";
import { createUsageScript, type Provider } from "@/types";
import { TEMPLATE_TYPES } from "@/config/constants";
import {
  extractCodexBaseUrl,
  extractCodexExperimentalBearerToken,
} from "@/utils/providerConfigUtils";

const IX_PROVIDER_ID = "default";
const IX_PROVIDER_NAME = "default";
const IX_CODE_BASE_URL = "https://code.gogoais.com";
const IX_KEY_ENDPOINT = "https://x-api.gogoais.com/api/public/codex-key";
const IX_CREDENTIALS_STORAGE_KEY = "codexSwitch:ixCredentials";
const IX_DEFAULT_PASSWORD = "123456";

type IxSavedCredentials = {
  account: string;
  password: string;
};

function loadIxSavedCredentials(): IxSavedCredentials {
  const fallback: IxSavedCredentials = {
    account: "",
    password: IX_DEFAULT_PASSWORD,
  };

  if (typeof window === "undefined") {
    return fallback;
  }

  try {
    const raw = window.localStorage.getItem(IX_CREDENTIALS_STORAGE_KEY);
    if (!raw) {
      return fallback;
    }

    const parsed = JSON.parse(raw) as Partial<IxSavedCredentials>;
    return {
      account: typeof parsed.account === "string" ? parsed.account : "",
      password:
        typeof parsed.password === "string"
          ? parsed.password
          : IX_DEFAULT_PASSWORD,
    };
  } catch (error) {
    console.warn("[IX] Failed to load saved credentials:", error);
    return fallback;
  }
}

function saveIxCredentials(credentials: IxSavedCredentials): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  try {
    window.localStorage.setItem(
      IX_CREDENTIALS_STORAGE_KEY,
      JSON.stringify(credentials),
    );
    return true;
  } catch (error) {
    console.warn("[IX] Failed to save credentials:", error);
    return false;
  }
}

const IX_USAGE_SCRIPT_CODE = `(() => {
  const timezone = "Asia/Shanghai";
  const pad = function (value) {
    const text = String(value);
    return text.length < 2 ? "0" + text : text;
  };
  const toShanghaiDate = function (date) {
    return new Date(date.getTime() + 8 * 60 * 60 * 1000);
  };
  const end = new Date();
  const shanghaiEnd = toShanghaiDate(end);
  const shanghaiStart = new Date(Date.UTC(
    shanghaiEnd.getUTCFullYear(),
    shanghaiEnd.getUTCMonth(),
    1
  ));
  const formatDate = function (date) {
    return date.getUTCFullYear() + "-" + pad(date.getUTCMonth() + 1) + "-" + pad(date.getUTCDate());
  };
  const startDate = formatDate(shanghaiStart);
  const endDate = formatDate(shanghaiEnd);

  return {
    request: {
      url: "{{baseUrl}}/usage?days=30&start_date=" + startDate + "&end_date=" + endDate + "&timezone=" + timezone,
      method: "GET",
      headers: {
        "Authorization": "Bearer {{apiKey}}",
        "Accept": "application/json",
        "User-Agent": "ccc-switch/1.0"
      }
    },
    extractor: function (response) {
      const root = response && (response.data || response.result || response);
      if (response && response.success === false) {
        return {
          isValid: false,
          invalidMessage: response.message || response.error || "IX 用量查询失败"
        };
      }
      const list =
        Array.isArray(root) ? root :
        root && Array.isArray(root.items) ? root.items :
        root && Array.isArray(root.keys) ? root.keys :
        root && Array.isArray(root.list) ? root.list :
        root && Array.isArray(root.records) ? root.records :
        null;
      const data = list && list.length > 0 ? list[0] : root || {};
      const firstNumber = function (source, keys) {
        for (let index = 0; index < keys.length; index += 1) {
          const key = keys[index];
          const value = source && source[key];
          if (typeof value === "number" && isFinite(value)) return value;
          if (typeof value === "string" && value.trim() !== "" && isFinite(Number(value))) {
            return Number(value);
          }
        }
        return undefined;
      };
      const numberOr = function (value, fallback) {
        return value === undefined ? fallback : value;
      };
      const normalizeUsdUnit = function (value) {
        const unit = value === undefined || value === null ? "" : String(value).trim();
        const lowerUnit = unit.toLowerCase();
        if (!unit || unit === "次" || unit === "美元" || unit === "$" || lowerUnit === "usd") {
          return "USD";
        }
        return unit;
      };
      const findObject = function (source, keys) {
        for (let index = 0; index < keys.length; index += 1) {
          const key = keys[index];
          const value = source && source[key];
          if (value && typeof value === "object") return value;
        }
        return undefined;
      };
      const firstRateLimit = function (source) {
        const limits = source && (source.rate_limits || source.rateLimits || source.limits);
        if (!Array.isArray(limits) || limits.length === 0) return {};
        for (let index = 0; index < limits.length; index += 1) {
          if (limits[index] && String(limits[index].window || "").toLowerCase() === "7d") {
            return limits[index];
          }
        }
        return limits[0] || {};
      };
      const resetFromWindowStart = function (windowStart, windowName) {
        if (!windowStart) return null;
        const startedAt = new Date(windowStart);
        if (isNaN(startedAt.getTime())) return null;
        const match = String(windowName || "").match(/^(\\d+)([dhm])$/);
        if (!match) return null;
        const count = Number(match[1]);
        const unit = match[2];
        const resetAt = new Date(startedAt.getTime());
        if (unit === "d") resetAt.setDate(resetAt.getDate() + count);
        if (unit === "h") resetAt.setHours(resetAt.getHours() + count);
        if (unit === "m") resetAt.setMinutes(resetAt.getMinutes() + count);
        return resetAt.toISOString();
      };
      const usage = findObject(data, ["usage"]) || data;
      const today = findObject(usage, ["today", "daily", "day", "current_day", "today_usage"]) || data;
      const period = findObject(usage, ["total", "last_30_days", "last30Days", "thirty_days", "month", "period"]) || usage;
      const quota = findObject(data, ["seven_day", "sevenDay", "weekly", "week", "quota", "rate_limit"]) || firstRateLimit(data);
      const todayUsed = numberOr(
        firstNumber(today, ["cost", "actual_cost", "today_usd", "todayUsd", "today_cost", "todayCost", "total_cost", "amount", "usd", "used", "value"]),
        0
      );
      const periodUsed = numberOr(
        firstNumber(period, ["cost", "actual_cost", "last_30_usd", "last30Usd", "last30_cost", "last30Cost", "total_cost", "amount", "usd", "used", "value"]),
        todayUsed
      );
      const quotaLimit = numberOr(firstNumber(quota, ["limit", "total", "quota", "max", "entitlement"]), 0);
      const quotaRemaining = numberOr(firstNumber(quota, ["remaining", "left", "available"]), Math.max(quotaLimit - numberOr(firstNumber(quota, ["used", "usage", "current"]), 0), 0));
      const quotaUsed = numberOr(firstNumber(quota, ["used", "usage", "current"]), Math.max(quotaLimit - quotaRemaining, 0));
      const quotaWindow = quota.window || quota.window_name || quota.name || "7d";
      const quotaUnit = normalizeUsdUnit(quota.unit || quota.currency || data.currency);
      const windowStart = quota.window_start || quota.windowStart || null;
      const resetsAt =
        quota.resets_at ||
        quota.reset_at ||
        quota.resetAt ||
        quota.next_reset_at ||
        quota.nextResetAt ||
        quota.reset_time ||
        quota.resetDate ||
        quota.quota_reset_at ||
        data.resets_at ||
        data.reset_at ||
        data.resetAt ||
        data.next_reset_at ||
        data.nextResetAt ||
        data.reset_time ||
        data.resetDate ||
        data.quota_reset_at ||
        resetFromWindowStart(windowStart, quotaWindow) ||
        null;
      const keyName = data.name || data.key_name || data.keyName || data.api_key_name || "default";
      const rawKeyValue = data.api_key || data.apiKey || data.key || data.sk || "{{apiKey}}";
      const rawKey = rawKeyValue ? String(rawKeyValue) : "";
      const maskedKey = rawKey && rawKey.length > 10
        ? rawKey.slice(0, 6) + "..." + rawKey.slice(-4)
        : "";
      const createdAt = data.created_at || data.createdAt || "";
      const updatedAt = data.updated_at || data.updatedAt || "";
      const expiresAt = data.expires_at || data.expiresAt || data.expire_at || "";
      const status = data.status || (data.active === false || data.is_active === false ? "inactive" : "active");
      const meta = {
        type: "ix_gogoai_usage",
        keyName: keyName,
        maskedKey: maskedKey,
        group: data.group || data.model_group || "codex",
        multiplier: data.multiplier || data.rate || "1x",
        todayUsd: todayUsed,
        last30Usd: periodUsed,
        quotaLabel: quotaWindow,
        quotaUsed: quotaUsed,
        quotaLimit: quotaLimit,
        quotaRemaining: quotaRemaining,
        quotaUnit: quotaUnit,
        quotaWindowStart: windowStart,
        resetsAt: resetsAt,
        expiresAt: expiresAt,
        status: status,
        daysUntilExpiry: data.days_until_expiry || data.daysUntilExpiry || null,
        mode: data.mode || "",
        createdAt: createdAt,
        updatedAt: updatedAt,
        startDate: startDate,
        endDate: endDate,
        timezone: timezone
      };

      return [
        {
          planName: "IX",
          used: periodUsed,
          unit: "USD",
          extra: JSON.stringify(meta)
        },
        {
          planName: "今日",
          used: todayUsed,
          unit: "USD"
        },
        {
          planName: "7d",
          total: quotaLimit,
          used: quotaUsed,
          remaining: quotaRemaining,
          unit: quotaUnit,
          extra: resetsAt || ""
        }
      ];
    }
  };
})()`;

function escapeTomlBasicString(value: string): string {
  return value.replace(/["\\\u0000-\u001f]/g, (ch) => {
    switch (ch) {
      case '"':
        return '\\"';
      case "\\":
        return "\\\\";
      case "\b":
        return "\\b";
      case "\t":
        return "\\t";
      case "\n":
        return "\\n";
      case "\f":
        return "\\f";
      case "\r":
        return "\\r";
      default:
        return `\\u${ch.charCodeAt(0).toString(16).padStart(4, "0")}`;
    }
  });
}

function buildCodexConfig(baseUrl: string, apiKey: string): string {
  const escapedBaseUrl = escapeTomlBasicString(baseUrl);
  const escapedApiKey = escapeTomlBasicString(apiKey);

  return `model_provider = "custom"
model = "gpt-5.5"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.custom]
name = "GogoAI"
base_url = "${escapedBaseUrl}"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "${escapedApiKey}"`;
}

function codexOpenaiBaseUrl(raw: string): string {
  const base = raw.trim().replace(/\/+$/, "");
  if (/\/v\d+$/i.test(base)) {
    return base;
  }
  return `${base}/v1`;
}

function createIxProvider(
  apiKey: string,
  baseUrl: string,
  existingProvider?: Provider,
): Provider {
  const usageScript = createIxUsageScript();
  const config = buildCodexConfig(baseUrl, apiKey);

  return {
    ...existingProvider,
    id: IX_PROVIDER_ID,
    name: IX_PROVIDER_NAME,
    websiteUrl: IX_CODE_BASE_URL,
    category: "third_party",
    icon: "default",
    iconColor: "#6B7280",
    settingsConfig: {
      apiKey,
      auth: {
        OPENAI_API_KEY: apiKey,
      },
      env: {
        OPENAI_API_KEY: apiKey,
      },
      config,
    },
    notes: existingProvider?.notes?.startsWith("IX 账号自动配置")
      ? undefined
      : existingProvider?.notes,
    meta: {
      ...(existingProvider?.meta ?? {}),
      providerType: "ix_gogoai",
      apiFormat: "openai_responses",
      endpointAutoSelect: false,
      usage_script: usageScript,
    },
    createdAt: existingProvider?.createdAt ?? Date.now(),
    sortIndex: existingProvider?.sortIndex,
  };
}

function mergeIxProviderDefaults(provider: Provider): Provider {
  const apiKey = resolveIxProviderApiKey(provider);
  const currentConfig =
    typeof provider.settingsConfig?.config === "string"
      ? provider.settingsConfig.config
      : "";
  const baseUrl = codexOpenaiBaseUrl(
    extractCodexBaseUrl(currentConfig) || IX_CODE_BASE_URL,
  );
  const settingsConfig = apiKey
    ? {
        ...(provider.settingsConfig ?? {}),
        apiKey,
        auth: {
          ...(provider.settingsConfig?.auth ?? {}),
          OPENAI_API_KEY: apiKey,
        },
        env: {
          ...(provider.settingsConfig?.env ?? {}),
          OPENAI_API_KEY: apiKey,
        },
        config: currentConfig || buildCodexConfig(baseUrl, apiKey),
      }
    : (provider.settingsConfig ?? {});

  return {
    ...provider,
    websiteUrl: provider.websiteUrl || IX_CODE_BASE_URL,
    category: provider.category || "third_party",
    settingsConfig,
    meta: {
      ...(provider.meta ?? {}),
      providerType: "ix_gogoai",
      apiFormat: "openai_responses",
      endpointAutoSelect: false,
      usage_script: createIxUsageScript(),
    },
  };
}

function resolveIxProviderApiKey(provider: Provider): string {
  const config = provider.settingsConfig ?? {};
  const candidates = [
    config.auth?.OPENAI_API_KEY,
    config.env?.OPENAI_API_KEY,
    config.apiKey,
    config.api_key,
    extractCodexExperimentalBearerToken(
      typeof config.config === "string" ? config.config : "",
    ),
  ];

  for (const candidate of candidates) {
    if (typeof candidate === "string" && candidate.trim()) {
      return candidate.trim();
    }
  }

  return "";
}

function createIxUsageScript() {
  return createUsageScript({
    enabled: true,
    language: "javascript",
    templateType: TEMPLATE_TYPES.CUSTOM,
    code: IX_USAGE_SCRIPT_CODE,
    timeout: 15,
    autoQueryInterval: 30,
  });
}

interface CodexIxQuickSetupProps {
  providers: Record<string, Provider>;
  onConfigured?: () => void | Promise<void>;
}

export function CodexIxQuickSetup({
  providers,
  onConfigured,
}: CodexIxQuickSetupProps) {
  const [credentials, setCredentials] = useState<IxSavedCredentials>(() =>
    loadIxSavedCredentials(),
  );
  const [showPassword, setShowPassword] = useState(false);
  const [credentialsSaveStatus, setCredentialsSaveStatus] = useState<
    "saved" | "saving" | "failed"
  >("saved");
  const [relayApiKey, setRelayApiKey] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isApplyingRelayKey, setIsApplyingRelayKey] = useState(false);
  const [isSyncingUsageScript, setIsSyncingUsageScript] = useState(false);
  const account = credentials.account;
  const password = credentials.password;
  const credentialsSaveMessage =
    credentialsSaveStatus === "saving"
      ? " 正在保存..."
      : credentialsSaveStatus === "failed"
        ? " 自动保存失败。"
        : " 已保存。";

  useEffect(() => {
    setCredentialsSaveStatus("saving");
    const timer = window.setTimeout(() => {
      setCredentialsSaveStatus(
        saveIxCredentials({
          account,
          password,
        })
          ? "saved"
          : "failed",
      );
    }, 250);

    return () => window.clearTimeout(timer);
  }, [account, password]);

  useEffect(() => {
    const provider = providers[IX_PROVIDER_ID];
    if (isSyncingUsageScript || provider?.meta?.providerType !== "ix_gogoai") {
      return;
    }

    const script = provider.meta?.usage_script;
    const apiKey = resolveIxProviderApiKey(provider);
    const hasCanonicalKey =
      !apiKey ||
      (provider.settingsConfig?.auth?.OPENAI_API_KEY === apiKey &&
        provider.settingsConfig?.env?.OPENAI_API_KEY === apiKey &&
        provider.settingsConfig?.apiKey === apiKey);
    if (
      hasCanonicalKey &&
      script?.enabled &&
      script.code === IX_USAGE_SCRIPT_CODE &&
      script.autoQueryInterval === 30
    ) {
      return;
    }

    setIsSyncingUsageScript(true);
    const nextProvider = mergeIxProviderDefaults(provider);

    providersApi
      .update(nextProvider, "codex", IX_PROVIDER_ID)
      .then(() => onConfigured?.())
      .catch((error) => {
        console.warn("[IX] Failed to sync usage script:", error);
      })
      .finally(() => setIsSyncingUsageScript(false));
  }, [isSyncingUsageScript, onConfigured, providers]);

  const handleSetup = useCallback(async () => {
    const normalizedAccount = account.trim();
    if (!normalizedAccount || !password) {
      toast.error("请输入 ix 账号密码");
      return;
    }

    setIsLoading(true);
    try {
      const result = await codexGogoaisApi.login({
        account: normalizedAccount,
        password,
        loginBaseUrl: IX_KEY_ENDPOINT,
        codeBaseUrl: IX_CODE_BASE_URL,
      });
      const existingProvider = providers[IX_PROVIDER_ID];
      const provider = createIxProvider(
        result.apiKey,
        result.baseUrl,
        existingProvider,
      );
      const exists = Boolean(existingProvider);

      if (exists) {
        await providersApi.update(provider, "codex", IX_PROVIDER_ID);
      } else {
        await providersApi.add(provider, "codex");
      }
      await providersApi.switch(IX_PROVIDER_ID, "codex");

      toast.success("已获取并配置 ix Codex 环境，用量查询已启用");
      await onConfigured?.();
    } catch (err) {
      console.warn("[IX] Codex quick setup failed:", err);
      toast.error(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "ix 账号配置失败，请检查账号密码",
      );
    } finally {
      setIsLoading(false);
    }
  }, [account, password, providers, onConfigured]);

  const handleApplyRelayKey = useCallback(async () => {
    const normalizedApiKey = relayApiKey.trim();
    if (!normalizedApiKey) {
      toast.error("请输入中转 API Key");
      return;
    }

    setIsApplyingRelayKey(true);
    try {
      const existingProvider = providers[IX_PROVIDER_ID];
      const provider = createIxProvider(
        normalizedApiKey,
        codexOpenaiBaseUrl(IX_CODE_BASE_URL),
        existingProvider,
      );
      const exists = Boolean(existingProvider);

      if (exists) {
        await providersApi.update(provider, "codex", IX_PROVIDER_ID);
      } else {
        await providersApi.add(provider, "codex");
      }
      await providersApi.switch(IX_PROVIDER_ID, "codex");

      setRelayApiKey("");
      toast.success("中转 API Key 已写入 default 环境并生效");
      await onConfigured?.();
    } catch (err) {
      console.warn("[IX] Codex relay API key setup failed:", err);
      toast.error(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "中转 API Key 配置失败，请检查后重试",
      );
    } finally {
      setIsApplyingRelayKey(false);
    }
  }, [relayApiKey, providers, onConfigured]);

  return (
    <div className="mt-5 space-y-3 px-6">
      <div className="flex items-center gap-2 text-sm font-medium text-foreground">
        <KeyRound className="h-4 w-4 text-muted-foreground" />
        IX Codex 快速配置
      </div>
      <Tabs defaultValue="account" className="space-y-3">
        <TabsList className="grid w-full max-w-md grid-cols-2">
          <TabsTrigger value="account">账号密码</TabsTrigger>
          <TabsTrigger value="apiKey">中转 API Key</TabsTrigger>
        </TabsList>

        <TabsContent value="account" className="mt-0 space-y-2">
          <div className="grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] md:items-end">
            <div className="space-y-1.5">
              <Label htmlFor="ix-account">IX 账号</Label>
              <Input
                id="ix-account"
                value={account}
                onChange={(event) =>
                  setCredentials((current) => ({
                    ...current,
                    account: event.target.value,
                  }))
                }
                placeholder="请输入账号"
                autoComplete="username"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ix-password">密码</Label>
              <div className="relative">
                <Input
                  id="ix-password"
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(event) =>
                    setCredentials((current) => ({
                      ...current,
                      password: event.target.value,
                    }))
                  }
                  placeholder="请输入密码"
                  autoComplete="current-password"
                  className="pr-10"
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void handleSetup();
                    }
                  }}
                />
                <TooltipProvider delayDuration={300}>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={() => setShowPassword((value) => !value)}
                        className="absolute inset-y-0 right-0 flex w-10 items-center justify-center rounded-r-md text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        aria-label={showPassword ? "隐藏密码" : "显示密码"}
                        title={showPassword ? "隐藏密码" : "显示密码"}
                      >
                        {showPassword ? (
                          <EyeOff className="h-4 w-4" />
                        ) : (
                          <Eye className="h-4 w-4" />
                        )}
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      {showPassword ? "隐藏已保存密码" : "查看已保存密码"}
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </div>
            </div>
            <TooltipProvider delayDuration={300}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    onClick={() => void handleSetup()}
                    disabled={isLoading}
                    aria-label="获取 API Key 并配置 IX Codex"
                    className="gap-1.5 whitespace-nowrap"
                  >
                    {isLoading ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <KeyRound className="h-4 w-4" />
                    )}
                    直接获取
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  获取 Key 并切换 default 环境
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p className="text-xs text-muted-foreground">
            账号密码会自动保存在本机；获取后自动配置 default 环境、切换到 https://code.gogoais.com/v1，并自动启用用量查询。
            {credentialsSaveMessage}
          </p>
        </TabsContent>

        <TabsContent value="apiKey" className="mt-0 space-y-2">
          <div className="grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
            <div className="space-y-1.5">
              <Label htmlFor="ix-relay-api-key">中转 API Key</Label>
              <Input
                id="ix-relay-api-key"
                type="password"
                value={relayApiKey}
                onChange={(event) => setRelayApiKey(event.target.value)}
                placeholder="sk-..."
                autoComplete="off"
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void handleApplyRelayKey();
                  }
                }}
              />
            </div>
            <TooltipProvider delayDuration={300}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    onClick={() => void handleApplyRelayKey()}
                    disabled={isApplyingRelayKey}
                    aria-label="写入中转 API Key 并立即生效"
                    className="gap-1.5 whitespace-nowrap"
                  >
                    {isApplyingRelayKey ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <ShieldCheck className="h-4 w-4" />
                    )}
                    直接生效
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  写入 Key 并切换 default 环境
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p className="text-xs text-muted-foreground">
            使用同一个中转地址 https://code.gogoais.com/v1；保存后会立即切换到 default 环境。
          </p>
        </TabsContent>
      </Tabs>
    </div>
  );
}
