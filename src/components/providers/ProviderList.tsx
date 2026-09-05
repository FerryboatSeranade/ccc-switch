import { CSS } from "@dnd-kit/utilities";
import { DndContext, closestCenter } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertTriangle,
  Code2,
  History,
  Loader2,
  RotateCcw,
  Search,
  Shuffle,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { Provider, Settings } from "@/types";
import type { AppId } from "@/lib/api";
import { providersApi } from "@/lib/api/providers";
import { settingsApi } from "@/lib/api/settings";
import { useSettingsQuery } from "@/lib/query";
import { subscriptionKeys } from "@/lib/query/subscription";
import { usageKeys } from "@/lib/query/usage";
import { extractErrorMessage } from "@/utils/errorUtils";
import { useDragSort } from "@/hooks/useDragSort";
import {
  useOpenClawLiveProviderIds,
  useOpenClawDefaultModel,
} from "@/hooks/useOpenClaw";
import {
  useHermesLiveProviderIds,
  useHermesModelConfig,
} from "@/hooks/useHermes";
import { useStreamCheck } from "@/hooks/useStreamCheck";
import { ProviderCard } from "@/components/providers/ProviderCard";
import { ProviderEmptyState } from "@/components/providers/ProviderEmptyState";
import { CodexIxQuickSetup } from "@/components/codex/CodexIxQuickSetup";
import {
  useAutoFailoverEnabled,
  useFailoverQueue,
  useAddToFailoverQueue,
  useRemoveFromFailoverQueue,
} from "@/lib/query/failover";
import {
  useCurrentOmoProviderId,
  useCurrentOmoSlimProviderId,
} from "@/lib/query/omo";
import { useCallback } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { isTextEditableTarget } from "@/utils/domUtils";
import { usePiCurrentState } from "@/lib/query/pi";
import { isProxyAppId } from "@/config/appConfig";

interface ProviderListProps {
  providers: Record<string, Provider>;
  currentProviderId: string;
  appId: AppId;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onRemoveFromConfig?: (provider: Provider) => void;
  onDisableOmo?: () => void;
  onDisableOmoSlim?: () => void;
  onDuplicate: (provider: Provider) => void;
  onConfigureUsage?: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  onCreate?: () => void;
  onRefresh?: () => void;
  isLoading?: boolean;
  isProxyRunning?: boolean; // 代理服务运行状态
  isProxyTakeover?: boolean; // 代理接管模式（Live配置已被接管）
  activeProviderId?: string; // 代理当前实际使用的供应商 ID（用于故障转移模式下标注绿色边框）
  onSetAsDefault?: (provider: Provider, modelId?: string) => void; // OpenClaw: set as default model
}

export function ProviderList({
  providers,
  currentProviderId,
  appId,
  onSwitch,
  onEdit,
  onDelete,
  onRemoveFromConfig,
  onDisableOmo,
  onDisableOmoSlim,
  onDuplicate,
  onConfigureUsage,
  onOpenWebsite,
  onOpenTerminal,
  onCreate,
  onRefresh,
  isLoading = false,
  isProxyRunning = false,
  isProxyTakeover = false,
  activeProviderId,
  onSetAsDefault,
}: ProviderListProps) {
  const { t } = useTranslation();
  const { checkProvider, isChecking } = useStreamCheck(appId);
  const { sortedProviders, sensors, handleDragEnd } = useDragSort(
    providers,
    appId,
  );

  const { data: opencodeLiveIds } = useQuery({
    queryKey: ["opencodeLiveProviderIds"],
    queryFn: () => providersApi.getOpenCodeLiveProviderIds(),
    enabled: appId === "opencode",
  });

  // OpenClaw: 查询 live 配置中的供应商 ID 列表，用于判断 isInConfig
  const { data: openclawLiveIds } = useOpenClawLiveProviderIds(
    appId === "openclaw",
  );

  // Hermes: 查询 live 配置中的供应商 ID 列表，用于判断 isInConfig
  const { data: hermesLiveIds } = useHermesLiveProviderIds(appId === "hermes");

  // Hermes: 读取当前 model.provider，用于判断哪个供应商是"当前激活"（高亮）
  const { data: hermesModelConfig } = useHermesModelConfig(appId === "hermes");
  const hermesCurrentProviderId = hermesModelConfig?.provider;

  // 判断供应商是否已添加到配置（累加模式应用：OpenCode/OpenClaw/Hermes）
  const isProviderInConfig = useCallback(
    (providerId: string): boolean => {
      if (appId === "opencode") {
        return opencodeLiveIds?.includes(providerId) ?? false;
      }
      if (appId === "openclaw") {
        return openclawLiveIds?.includes(providerId) ?? false;
      }
      if (appId === "hermes") {
        return hermesLiveIds?.includes(providerId) ?? false;
      }
      return true; // 其他应用始终返回 true
    },
    [appId, opencodeLiveIds, openclawLiveIds, hermesLiveIds],
  );

  // OpenClaw: query default model to determine which provider is default
  const { data: openclawDefaultModel } = useOpenClawDefaultModel(
    appId === "openclaw",
  );

  const isProviderDefaultModel = useCallback(
    (providerId: string): boolean => {
      if (appId !== "openclaw" || !openclawDefaultModel?.primary) return false;
      return openclawDefaultModel.primary.startsWith(providerId + "/");
    },
    [appId, openclawDefaultModel],
  );

  // Only apps with an explicit local-routing capability participate in
  // failover. Additive apps such as Pi never query or render this state.
  const supportsFailover = isProxyAppId(appId);
  const { data: isAutoFailoverEnabled } = useAutoFailoverEnabled(
    appId,
    supportsFailover,
  );
  const { data: failoverQueue } = useFailoverQueue(appId, supportsFailover);
  const addToQueue = useAddToFailoverQueue();
  const removeFromQueue = useRemoveFromFailoverQueue();

  const isFailoverModeActive =
    supportsFailover &&
    isProxyTakeover === true &&
    isAutoFailoverEnabled === true;

  const isOpenCode = appId === "opencode";
  const { data: currentOmoId } = useCurrentOmoProviderId(isOpenCode);
  const { data: currentOmoSlimId } = useCurrentOmoSlimProviderId(isOpenCode);

  const getFailoverPriority = useCallback(
    (providerId: string): number | undefined => {
      if (!isFailoverModeActive || !failoverQueue) return undefined;
      const index = failoverQueue.findIndex(
        (item) => item.providerId === providerId,
      );
      return index >= 0 ? index + 1 : undefined;
    },
    [isFailoverModeActive, failoverQueue],
  );

  const isInFailoverQueue = useCallback(
    (providerId: string): boolean => {
      if (!isFailoverModeActive || !failoverQueue) return false;
      return failoverQueue.some((item) => item.providerId === providerId);
    },
    [isFailoverModeActive, failoverQueue],
  );

  const handleToggleFailover = useCallback(
    (providerId: string, enabled: boolean) => {
      if (enabled) {
        addToQueue.mutate({ appType: appId, providerId });
      } else {
        removeFromQueue.mutate({ appType: appId, providerId });
      }
    },
    [appId, addToQueue, removeFromQueue],
  );

  const [searchTerm, setSearchTerm] = useState("");
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isRestartingApp, setIsRestartingApp] = useState(false);
  const [isRestartingVsCode, setIsRestartingVsCode] = useState(false);
  const [isSavingUnifyHistory, setIsSavingUnifyHistory] = useState(false);
  const [isMigratingUnifiedHistory, setIsMigratingUnifiedHistory] =
    useState(false);
  const [showUnifyEnableConfirm, setShowUnifyEnableConfirm] = useState(false);
  const [showUnifyDisableConfirm, setShowUnifyDisableConfirm] =
    useState(false);
  const [hasUnifyBackup, setHasUnifyBackup] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const { data: settings } = useSettingsQuery();
  const { data: claudeDesktopStatus } = useQuery({
    queryKey: ["claudeDesktopStatus"],
    queryFn: () => providersApi.getClaudeDesktopStatus(),
    enabled: appId === "claude-desktop",
    refetchInterval: appId === "claude-desktop" ? 5000 : false,
  });
  const {
    data: piCurrentState,
    isSuccess: isPiCurrentStateSuccess,
    isError: isPiCurrentStateError,
    error: piCurrentStateError,
  } = usePiCurrentState(appId === "pi");
  const isPiAuthoritativeStateReady = appId !== "pi" || isPiCurrentStateSuccess;
  const isPiProviderInConfig = useCallback(
    (provider: Provider): boolean => {
      if (!isPiAuthoritativeStateReady) return false;
      return piCurrentState?.enabledProviderIds.includes(provider.id) ?? false;
    },
    [isPiAuthoritativeStateReady, piCurrentState],
  );

  // 连通性检查不发真实请求、无封号/计费风险，直接执行（无需确认弹窗）。
  const handleTest = useCallback(
    (provider: Provider) => {
      checkProvider(provider.id, provider.name);
    },
    [checkProvider],
  );

  // Import current live config as default provider
  const queryClient = useQueryClient();
  const importMutation = useMutation({
    mutationFn: async (): Promise<boolean> => {
      if (appId === "opencode") {
        const count = await providersApi.importOpenCodeFromLive();
        return count > 0;
      }
      if (appId === "openclaw") {
        const count = await providersApi.importOpenClawFromLive();
        return count > 0;
      }
      if (appId === "hermes") {
        const count = await providersApi.importHermesFromLive();
        return count > 0;
      }
      if (appId === "claude-desktop") {
        const count = await providersApi.importClaudeDesktopFromClaude();
        return count > 0;
      }
      return providersApi.importDefault(appId);
    },
    onSuccess: (imported) => {
      if (imported) {
        queryClient.invalidateQueries({ queryKey: ["providers", appId] });
        if (appId === "claude-desktop") {
          queryClient.invalidateQueries({ queryKey: ["claudeDesktopStatus"] });
        }
        toast.success(t("provider.importCurrentDescription"));
      } else {
        toast.info(t("provider.noProviders"));
      }
    },
    onError: (error: unknown) => {
      // Tauri invoke 的 reject 值是后端序列化出的纯字符串而非 Error 对象，
      // 取 .message 只会得到 undefined（空 toast）。
      toast.error(extractErrorMessage(error) || t("settings.importFailed"));
      // 导入失败前也可能已产生需要上屏的副作用：GrokBuild 官方登录态下点
      // 导入，命令层会先补种官方条目、随后才因 live 不可导入而报错。
      queryClient.invalidateQueries({ queryKey: ["providers", appId] });
    },
  });

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;

      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "f") {
        // 正在输入框/可编辑区域中时不抢占 Ctrl+F（例如添加供应商表单里
        // ProviderPresetSelector 的搜索框），避免与其同名快捷键冲突。
        if (isTextEditableTarget(document.activeElement)) return;
        event.preventDefault();
        setIsSearchOpen(true);
        return;
      }

      if (key === "escape") {
        setIsSearchOpen(false);
      }
    };

    globalThis.addEventListener("keydown", handleKeyDown);
    return () => globalThis.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (isSearchOpen) {
      const frame = requestAnimationFrame(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [isSearchOpen]);

  const filteredProviders = useMemo(() => {
    const keyword = searchTerm.trim().toLowerCase();
    if (!keyword) return sortedProviders;
    return sortedProviders.filter((provider) => {
      const fields = [provider.name, provider.notes, provider.websiteUrl];
      return fields.some((field) =>
        field?.toString().toLowerCase().includes(keyword),
      );
    });
  }, [searchTerm, sortedProviders]);

  const claudeDesktopStatusMessages = useMemo(() => {
    if (appId !== "claude-desktop" || !claudeDesktopStatus) return [];

    const messages: string[] = [];
    if (!claudeDesktopStatus.supported) {
      messages.push(
        t("claudeDesktop.statusUnsupported", {
          defaultValue: "当前平台暂不支持 Claude Desktop 3P 配置写入。",
        }),
      );
      return messages;
    }

    if (claudeDesktopStatus.staleRawModels) {
      messages.push(
        t("claudeDesktop.statusStaleRawModels", {
          defaultValue:
            "Claude Desktop profile 中存在非 claude-* 模型名，新版 Claude Desktop 可能拒绝加载；重新切换当前供应商可修复。",
        }),
      );
    }
    if (claudeDesktopStatus.missingRouteMappings) {
      messages.push(
        t("claudeDesktop.statusMissingRouteMappings", {
          defaultValue:
            "当前供应商启用了模型映射，但没有有效路由；请编辑供应商并补全至少一个模型映射。",
        }),
      );
    }
    if (
      claudeDesktopStatus.mode === "proxy" &&
      !claudeDesktopStatus.gatewayTokenConfigured
    ) {
      messages.push(
        t("claudeDesktop.statusGatewayTokenMissing", {
          defaultValue:
            "当前本地路由 token 尚未生成；重新切换该供应商会写入新的本地 token。",
        }),
      );
    }

    const expected = claudeDesktopStatus.expectedBaseUrl?.replace(/\/+$/, "");
    const actual = claudeDesktopStatus.actualBaseUrl?.replace(/\/+$/, "");
    if (expected && actual && expected !== actual) {
      messages.push(
        t("claudeDesktop.statusBaseUrlMismatch", {
          expected,
          actual,
          defaultValue:
            "Claude Desktop profile 指向的地址与当前供应商不一致；当前为 {{actual}}，应为 {{expected}}。重新切换当前供应商可修复。",
        }),
      );
    }

    return messages;
  }, [appId, claudeDesktopStatus, t]);

  const handleCodexQuickConfigured = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["providers", appId] }),
      queryClient.invalidateQueries({ queryKey: usageKeys.script("default", appId) }),
      queryClient.invalidateQueries({ queryKey: subscriptionKeys.quota(appId) }),
      queryClient.invalidateQueries({ queryKey: ["codex_oauth", "quota"] }),
    ]);
    onRefresh?.();
  }, [appId, onRefresh, queryClient]);

  const handleRestartApp = useCallback(async () => {
    if (isRestartingApp) return;

    setIsRestartingApp(true);
    toast.info(
      t("provider.restartAppStarting", {
        defaultValue: "Restarting ChatGPT...",
      }),
    );
    try {
      await settingsApi.restartChatGPTApp();
      toast.success(
        t("provider.restartAppSuccess", {
          defaultValue: "ChatGPT restarted",
        }),
      );
    } catch (error) {
      console.error("[ProviderList] Failed to restart ChatGPT", error);
      const message =
        typeof error === "string"
          ? error
          : error instanceof Error
            ? error.message
            : t("provider.restartAppFailed", {
                defaultValue: "Failed to restart ChatGPT",
              });
      toast.error(
        message ||
          t("provider.restartAppFailed", {
            defaultValue: "Failed to restart ChatGPT",
          }),
      );
    } finally {
      setIsRestartingApp(false);
    }
  }, [isRestartingApp, t]);

  const handleRestartVsCode = useCallback(async () => {
    if (isRestartingVsCode) return;

    setIsRestartingVsCode(true);
    toast.info(
      t("provider.restartVsCodeStarting", {
        defaultValue: "Restarting VS Code...",
      }),
    );
    try {
      await settingsApi.restartVsCode();
      toast.success(
        t("provider.restartVsCodeSuccess", {
          defaultValue: "VS Code restarted",
        }),
      );
    } catch (error) {
      console.error("[ProviderList] Failed to restart VS Code", error);
      const message =
        typeof error === "string"
          ? error
          : error instanceof Error
            ? error.message
            : t("provider.restartVsCodeFailed", {
                defaultValue: "Failed to restart VS Code",
              });
      toast.error(
        message ||
          t("provider.restartVsCodeFailed", {
            defaultValue: "Failed to restart VS Code",
          }),
      );
    } finally {
      setIsRestartingVsCode(false);
    }
  }, [isRestartingVsCode, t]);

  const unifyCodexSessionHistory = settings?.unifyCodexSessionHistory ?? true;
  const showRestoreUnifyOption =
    hasUnifyBackup || (settings?.unifyCodexMigrateExisting ?? false);

  const saveUnifyCodexHistory = useCallback(
    async (updates: Partial<Settings>): Promise<boolean> => {
      if (!settings || isSavingUnifyHistory) return false;

      setIsSavingUnifyHistory(true);
      try {
        await settingsApi.save({ ...settings, ...updates });
        await queryClient.invalidateQueries({ queryKey: ["settings"] });
        toast.success(
          updates.unifyCodexSessionHistory
            ? t("provider.unifySessionEnabled", {
                defaultValue: "Unified session history enabled",
              })
            : t("provider.unifySessionDisabled", {
                defaultValue: "Unified session history disabled",
              }),
        );
        return true;
      } catch (error) {
        console.error("[ProviderList] Failed to update unified history", error);
        toast.error(
          t("provider.unifySessionSaveFailed", {
            defaultValue: "Failed to update unified session history",
          }),
        );
        return false;
      } finally {
        setIsSavingUnifyHistory(false);
      }
    },
    [isSavingUnifyHistory, queryClient, settings, t],
  );

  const handleUnifySessionClick = useCallback(() => {
    if (isSavingUnifyHistory) return;

    if (unifyCodexSessionHistory) {
      void settingsApi
        .hasCodexUnifyHistoryBackup()
        .catch(() => false)
        .then((hasBackup) => {
          setHasUnifyBackup(hasBackup);
          setShowUnifyDisableConfirm(true);
        });
      return;
    }

    setShowUnifyEnableConfirm(true);
  }, [isSavingUnifyHistory, unifyCodexSessionHistory]);

  const handleUnifyEnableConfirm = useCallback(
    (migrateExisting: boolean) => {
      setShowUnifyEnableConfirm(false);
      void saveUnifyCodexHistory({
        unifyCodexSessionHistory: true,
        unifyCodexMigrateExisting: migrateExisting,
      });
    },
    [saveUnifyCodexHistory],
  );

  const handleUnifyDisableConfirm = useCallback(
    async (restoreBackup: boolean) => {
      setShowUnifyDisableConfirm(false);
      const saved = await saveUnifyCodexHistory({
        unifyCodexSessionHistory: false,
        unifyCodexMigrateExisting: false,
      });
      if (!saved || !restoreBackup) return;

      try {
        const result = await settingsApi.restoreCodexUnifiedHistory();
        if (result.skippedReason) {
          toast.info(
            result.skippedReason === "unify_toggle_on"
              ? t("settings.unifyCodexHistoryRestoreSkippedToggleOn")
              : t("settings.unifyCodexHistoryRestoreNothing"),
          );
          return;
        }
        toast.success(
          t("settings.unifyCodexHistoryRestoreCompleted", {
            files: result.restoredJsonlFiles,
            rows: result.restoredStateRows,
          }),
        );
      } catch (error) {
        console.error("Failed to restore codex unified history:", error);
        toast.error(t("settings.unifyCodexHistoryRestoreFailed"));
      }
    },
    [saveUnifyCodexHistory, t],
  );

  const handleMigrateUnifiedHistory = useCallback(async () => {
    if (
      isMigratingUnifiedHistory ||
      isSavingUnifyHistory ||
      !unifyCodexSessionHistory
    ) {
      return;
    }

    setIsMigratingUnifiedHistory(true);
    toast.info(
      t("provider.unifyHistoryMigrationStarting", {
        defaultValue: "正在同步 Codex 历史到统一桶...",
      }),
    );
    try {
      const result = await settingsApi.migrateCodexUnifiedHistory();
      await queryClient.invalidateQueries({ queryKey: ["settings"] });

      if (result.skippedReason) {
        const messageKey =
          result.skippedReason === "already_migrated"
            ? "provider.unifyHistoryMigrationAlreadyDone"
            : result.skippedReason === "live_not_unified"
              ? "provider.unifyHistoryMigrationLiveNotUnified"
              : result.skippedReason === "unify_toggle_off"
                ? "provider.unifyHistoryMigrationToggleOff"
                : "provider.unifyHistoryMigrationSkipped";
        toast.info(
          t(messageKey, {
            reason: result.skippedReason,
            defaultValue: `同步已跳过：${result.skippedReason}`,
          }),
        );
        return;
      }

      toast.success(
        t("provider.unifyHistoryMigrationCompleted", {
          files: result.migratedJsonlFiles,
          rows: result.migratedStateRows,
          defaultValue: `已同步历史：${result.migratedJsonlFiles} 个会话文件、${result.migratedStateRows} 条索引记录`,
        }),
      );
    } catch (error) {
      console.error("[ProviderList] Failed to migrate unified history", error);
      toast.error(
        t("provider.unifyHistoryMigrationFailed", {
          defaultValue: "同步 Codex 历史到统一桶失败",
        }),
      );
    } finally {
      setIsMigratingUnifiedHistory(false);
    }
  }, [
    isMigratingUnifiedHistory,
    isSavingUnifyHistory,
    queryClient,
    t,
    unifyCodexSessionHistory,
  ]);

  const codexQuickSetup =
    appId === "codex" ? (
      <CodexIxQuickSetup
        providers={providers}
        onConfigured={handleCodexQuickConfigured}
      />
    ) : null;

  const piStateErrorMessages = [
    isPiCurrentStateError ? extractErrorMessage(piCurrentStateError) : "",
  ].filter(Boolean);
  const piStateErrorNotice =
    appId === "pi" && piStateErrorMessages.length > 0 ? (
      <div
        role="alert"
        className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-900 dark:text-amber-200"
      >
        <div className="flex items-center gap-2 font-medium">
          <AlertTriangle className="h-4 w-4 shrink-0" />
          {t("pi.current.readFailed", {
            defaultValue: "无法读取 Pi 当前配置",
          })}
        </div>
        <p className="mt-1 text-xs leading-relaxed">
          {t("pi.current.stateUnavailableHint")}
          {piStateErrorMessages.length > 0
            ? ` ${piStateErrorMessages.join(" · ")}`
            : ""}
        </p>
      </div>
    ) : null;

  if (isLoading) {
    return (
      <div className="space-y-3">
        {[0, 1, 2].map((index) => (
          <div
            key={index}
            className="w-full border border-dashed rounded-lg h-28 border-muted-foreground/40 bg-muted/40"
          />
        ))}
      </div>
    );
  }

  if (sortedProviders.length === 0) {
    return (
      <div className="mt-4 space-y-4">
        {piStateErrorNotice}
        <ProviderEmptyState
          appId={appId}
          onCreate={appId === "pi" ? undefined : onCreate}
          onImport={appId === "pi" ? undefined : () => importMutation.mutate()}
        />
        {codexQuickSetup}
      </div>
    );
  }

  const renderProviderList = () => (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={handleDragEnd}
    >
      <SortableContext
        items={filteredProviders.map((provider) => provider.id)}
        strategy={verticalListSortingStrategy}
      >
        <div className="space-y-3">
          {filteredProviders.map((provider) => {
            const isOmo = provider.category === "omo";
            const isOmoSlim = provider.category === "omo-slim";
            const isOmoCurrent = isOmo && provider.id === (currentOmoId || "");
            const isOmoSlimCurrent =
              isOmoSlim && provider.id === (currentOmoSlimId || "");
            const isHermesCurrent =
              appId === "hermes" && hermesCurrentProviderId === provider.id;
            const isCurrent =
              appId === "pi"
                ? false
                : isOmo
                  ? isOmoCurrent
                  : isOmoSlim
                    ? isOmoSlimCurrent
                    : appId === "hermes"
                      ? isHermesCurrent
                      : provider.id === currentProviderId;
            return (
              <SortableProviderCard
                key={provider.id}
                provider={provider}
                isCurrent={isCurrent}
                appId={appId}
                isInConfig={
                  appId === "pi"
                    ? isPiProviderInConfig(provider)
                    : isProviderInConfig(provider.id)
                }
                isOmo={isOmo}
                isOmoSlim={isOmoSlim}
                onSwitch={onSwitch}
                onEdit={onEdit}
                onDelete={onDelete}
                onRemoveFromConfig={onRemoveFromConfig}
                onDisableOmo={onDisableOmo}
                onDisableOmoSlim={onDisableOmoSlim}
                onDuplicate={onDuplicate}
                onConfigureUsage={onConfigureUsage}
                onOpenWebsite={onOpenWebsite}
                onOpenTerminal={onOpenTerminal}
                onTest={handleTest}
                isTesting={isChecking(provider.id)}
                isProxyRunning={supportsFailover && isProxyRunning}
                isProxyTakeover={supportsFailover && isProxyTakeover}
                isAutoFailoverEnabled={isFailoverModeActive}
                failoverPriority={getFailoverPriority(provider.id)}
                isInFailoverQueue={isInFailoverQueue(provider.id)}
                onToggleFailover={
                  supportsFailover
                    ? (enabled) => handleToggleFailover(provider.id, enabled)
                    : undefined
                }
                activeProviderId={
                  supportsFailover ? activeProviderId : undefined
                }
                isDefaultModel={
                  appId === "hermes"
                    ? isHermesCurrent
                    : isProviderDefaultModel(provider.id)
                }
                isRemovalProtected={
                  appId === "pi"
                    ? false
                    : appId === "hermes"
                      ? isHermesCurrent
                      : appId === "openclaw"
                        ? isProviderDefaultModel(provider.id)
                        : false
                }
                isStateChangeProtected={
                  appId === "pi" && !isPiAuthoritativeStateReady
                }
                onSetAsDefault={
                  onSetAsDefault
                    ? (modelId) => onSetAsDefault(provider, modelId)
                    : undefined
                }
              />
            );
          })}
        </div>
      </SortableContext>
    </DndContext>
  );

  return (
    <div className="mt-4 space-y-4">
      {appId === "codex" && (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <TooltipProvider delayDuration={300}>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant={unifyCodexSessionHistory ? "secondary" : "outline"}
                  size="sm"
                  className={cn(
                    "h-8 gap-1.5 text-xs",
                    unifyCodexSessionHistory &&
                      "border-sky-500/30 bg-sky-500/10 text-sky-700 hover:bg-sky-500/15 dark:text-sky-300",
                  )}
                  onClick={handleUnifySessionClick}
                  disabled={!settings || isSavingUnifyHistory}
                  title={t("provider.unifySessionTooltip", {
                    defaultValue:
                      "Use one Codex session history for official and third-party providers",
                  })}
                  aria-label={t("provider.unifySessionTooltip", {
                    defaultValue:
                      "Use one Codex session history for official and third-party providers",
                  })}
                >
                  {isSavingUnifyHistory ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <History className="h-3.5 w-3.5" />
                  )}
                  {unifyCodexSessionHistory
                    ? t("provider.unifySessionOn", {
                        defaultValue: "Unified Sessions On",
                      })
                    : t("provider.unifySession", {
                        defaultValue: "Unify Sessions",
                      })}
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t("provider.unifySessionTooltip", {
                  defaultValue:
                    "Use one Codex session history for official and third-party providers",
                })}
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 text-xs"
                  onClick={handleMigrateUnifiedHistory}
                  disabled={
                    !settings ||
                    !unifyCodexSessionHistory ||
                    isSavingUnifyHistory ||
                    isMigratingUnifiedHistory
                  }
                  title={t("provider.unifyHistoryMigrationTooltip", {
                    defaultValue:
                      "Sync existing Codex history into the unified bucket",
                  })}
                  aria-label={t("provider.unifyHistoryMigrationTooltip", {
                    defaultValue:
                      "Sync existing Codex history into the unified bucket",
                  })}
                >
                  {isMigratingUnifiedHistory ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Shuffle className="h-3.5 w-3.5" />
                  )}
                  {isMigratingUnifiedHistory
                    ? t("provider.unifyHistoryMigrating", {
                        defaultValue: "Syncing",
                      })
                    : t("provider.unifyHistoryMigration", {
                        defaultValue: "Sync History",
                      })}
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {unifyCodexSessionHistory
                  ? t("provider.unifyHistoryMigrationTooltip", {
                      defaultValue:
                        "Sync existing Codex history into the unified bucket",
                    })
                  : t("provider.unifyHistoryMigrationDisabledTooltip", {
                      defaultValue:
                        "Enable unified sessions before syncing history",
                    })}
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 text-xs"
                  onClick={handleRestartApp}
                  disabled={isRestartingApp}
                  title={t("provider.restartAppTooltip", {
                    defaultValue:
                      "Restart ChatGPT, which now contains Codex. Falls back to the legacy Codex App.",
                  })}
                  aria-label={t("provider.restartAppTooltip", {
                    defaultValue:
                      "Restart ChatGPT, which now contains Codex. Falls back to the legacy Codex App.",
                  })}
                >
                  {isRestartingApp ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RotateCcw className="h-3.5 w-3.5" />
                  )}
                  {isRestartingApp
                    ? t("provider.restartingApp", {
                        defaultValue: "Restarting",
                      })
                    : t("provider.restartApp", {
                        defaultValue: "Restart ChatGPT (Codex)",
                      })}
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t("provider.restartAppTooltip", {
                  defaultValue:
                    "Restart ChatGPT, which now contains Codex. Falls back to the legacy Codex App.",
                })}
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 text-xs"
                  onClick={handleRestartVsCode}
                  disabled={isRestartingVsCode}
                  title={t("provider.restartVsCodeTooltip", {
                    defaultValue:
                      "Restart VS Code so the Codex extension reloads local configuration and authentication.",
                  })}
                  aria-label={t("provider.restartVsCodeTooltip", {
                    defaultValue:
                      "Restart VS Code so the Codex extension reloads local configuration and authentication.",
                  })}
                >
                  {isRestartingVsCode ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Code2 className="h-3.5 w-3.5" />
                  )}
                  {isRestartingVsCode
                    ? t("provider.restartingApp", {
                        defaultValue: "Restarting",
                      })
                    : t("provider.restartVsCode", {
                        defaultValue: "Restart VS Code",
                      })}
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t("provider.restartVsCodeTooltip", {
                  defaultValue:
                    "Restart VS Code so the Codex extension reloads local configuration and authentication.",
                })}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>
      )}

      <ConfirmDialog
        isOpen={showUnifyEnableConfirm}
        title={t("confirm.unifyCodexHistory.title")}
        message={t("confirm.unifyCodexHistory.message")}
        checkboxLabel={t("confirm.unifyCodexHistory.migrateExisting")}
        confirmText={t("confirm.unifyCodexHistory.confirm")}
        onConfirm={handleUnifyEnableConfirm}
        onCancel={() => setShowUnifyEnableConfirm(false)}
      />

      <ConfirmDialog
        isOpen={showUnifyDisableConfirm}
        title={t("confirm.unifyCodexHistoryOff.title")}
        message={t("confirm.unifyCodexHistoryOff.message")}
        checkboxLabel={
          showRestoreUnifyOption
            ? t("confirm.unifyCodexHistoryOff.restoreBackup")
            : undefined
        }
        checkboxDefaultChecked
        confirmText={t("confirm.unifyCodexHistoryOff.confirm")}
        onConfirm={(restoreBackup) =>
          void handleUnifyDisableConfirm(restoreBackup)
        }
        onCancel={() => setShowUnifyDisableConfirm(false)}
      />

      {piStateErrorNotice}
      {claudeDesktopStatusMessages.length > 0 && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-900 dark:text-amber-200">
          <div className="flex items-center gap-2 font-medium">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            {t("claudeDesktop.statusTitle", {
              defaultValue: "Claude Desktop 配置需要检查",
            })}
          </div>
          <ul className="mt-2 space-y-1 text-xs leading-relaxed">
            {claudeDesktopStatusMessages.map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </div>
      )}
      <AnimatePresence>
        {isSearchOpen && (
          <motion.div
            key="provider-search"
            initial={{ opacity: 0, y: -8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.98 }}
            transition={{ duration: 0.18, ease: "easeOut" }}
            className="fixed left-1/2 top-[6.5rem] z-40 w-[min(90vw,26rem)] -translate-x-1/2 sm:right-6 sm:left-auto sm:translate-x-0"
          >
            <div className="p-4 space-y-3 border shadow-md rounded-2xl border-white/10 bg-background/95 shadow-black/20 backdrop-blur-md">
              <div className="relative flex items-center gap-2">
                <Search className="absolute w-4 h-4 -translate-y-1/2 pointer-events-none left-3 top-1/2 text-muted-foreground" />
                <Input
                  ref={searchInputRef}
                  value={searchTerm}
                  onChange={(event) => setSearchTerm(event.target.value)}
                  placeholder={t("provider.searchPlaceholder", {
                    defaultValue: "Search name, notes, or URL...",
                  })}
                  aria-label={t("provider.searchAriaLabel", {
                    defaultValue: "Search providers",
                  })}
                  className="pr-16 pl-9"
                />
                {searchTerm && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="absolute text-xs -translate-y-1/2 right-11 top-1/2"
                    onClick={() => setSearchTerm("")}
                  >
                    {t("common.clear", { defaultValue: "Clear" })}
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  className="ml-auto"
                  onClick={() => setIsSearchOpen(false)}
                  aria-label={t("provider.searchCloseAriaLabel", {
                    defaultValue: "Close provider search",
                  })}
                >
                  <X className="w-4 h-4" />
                </Button>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
                <span>
                  {t("provider.searchScopeHint", {
                    defaultValue: "Matches provider name, notes, and URL.",
                  })}
                </span>
                <span>
                  {t("provider.searchCloseHint", {
                    defaultValue: "Press Esc to close",
                  })}
                </span>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {filteredProviders.length === 0 ? (
        <div className="px-6 py-8 text-sm text-center border border-dashed rounded-lg border-border text-muted-foreground">
          {t("provider.noSearchResults", {
            defaultValue: "No providers match your search.",
          })}
        </div>
      ) : (
        renderProviderList()
      )}
      {codexQuickSetup}
    </div>
  );
}

interface SortableProviderCardProps {
  provider: Provider;
  isCurrent: boolean;
  appId: AppId;
  isInConfig: boolean;
  isOmo: boolean;
  isOmoSlim: boolean;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onRemoveFromConfig?: (provider: Provider) => void;
  onDisableOmo?: () => void;
  onDisableOmoSlim?: () => void;
  onDuplicate: (provider: Provider) => void;
  onConfigureUsage?: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  onTest?: (provider: Provider) => void;
  isTesting: boolean;
  isProxyRunning: boolean;
  isProxyTakeover: boolean;
  isAutoFailoverEnabled: boolean;
  failoverPriority?: number;
  isInFailoverQueue: boolean;
  onToggleFailover?: (enabled: boolean) => void;
  activeProviderId?: string;
  // OpenClaw: default model
  isDefaultModel?: boolean;
  isRemovalProtected?: boolean;
  isStateChangeProtected?: boolean;
  onSetAsDefault?: (modelId?: string) => void;
}

function SortableProviderCard({
  provider,
  isCurrent,
  appId,
  isInConfig,
  isOmo,
  isOmoSlim,
  onSwitch,
  onEdit,
  onDelete,
  onRemoveFromConfig,
  onDisableOmo,
  onDisableOmoSlim,
  onDuplicate,
  onConfigureUsage,
  onOpenWebsite,
  onOpenTerminal,
  onTest,
  isTesting,
  isProxyRunning,
  isProxyTakeover,
  isAutoFailoverEnabled,
  failoverPriority,
  isInFailoverQueue,
  onToggleFailover,
  activeProviderId,
  isDefaultModel,
  isRemovalProtected,
  isStateChangeProtected,
  onSetAsDefault,
}: SortableProviderCardProps) {
  const {
    setNodeRef,
    attributes,
    listeners,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });

  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div ref={setNodeRef} style={style}>
      <ProviderCard
        provider={provider}
        isCurrent={isCurrent}
        appId={appId}
        isInConfig={isInConfig}
        isOmo={isOmo}
        isOmoSlim={isOmoSlim}
        onSwitch={onSwitch}
        onEdit={onEdit}
        onDelete={onDelete}
        onRemoveFromConfig={onRemoveFromConfig}
        onDisableOmo={onDisableOmo}
        onDisableOmoSlim={onDisableOmoSlim}
        onDuplicate={onDuplicate}
        onConfigureUsage={
          onConfigureUsage ? (item) => onConfigureUsage(item) : () => undefined
        }
        onOpenWebsite={onOpenWebsite}
        onOpenTerminal={onOpenTerminal}
        onTest={onTest}
        isTesting={isTesting}
        isProxyRunning={isProxyRunning}
        isProxyTakeover={isProxyTakeover}
        dragHandleProps={{
          attributes,
          listeners,
          isDragging,
        }}
        isAutoFailoverEnabled={isAutoFailoverEnabled}
        failoverPriority={failoverPriority}
        isInFailoverQueue={isInFailoverQueue}
        onToggleFailover={onToggleFailover}
        activeProviderId={activeProviderId}
        // OpenClaw: default model
        isDefaultModel={isDefaultModel}
        isRemovalProtected={isRemovalProtected}
        isStateChangeProtected={isStateChangeProtected}
        onSetAsDefault={onSetAsDefault}
      />
    </div>
  );
}
