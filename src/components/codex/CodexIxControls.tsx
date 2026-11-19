import { useCallback, useState } from "react";
import { Code2, History, Loader2, RotateCcw, Shuffle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "@/lib/toast";
import type { Provider, Settings } from "@/types";
import { settingsApi } from "@/lib/api/settings";
import { useSettingsQuery } from "@/lib/query";
import { subscriptionKeys } from "@/lib/query/subscription";
import { usageKeys } from "@/lib/query/usage";
import { CodexIxQuickSetup } from "./CodexIxQuickSetup";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";

export function CodexIxControls({ providers }: { providers: Record<string, Provider> }) {
 const appId = "codex";
 const {t}=useTranslation();
 const queryClient=useQueryClient();
 const {data: settings}=useSettingsQuery();
  const [isRestartingApp, setIsRestartingApp] = useState(false);
  const [isRestartingVsCode, setIsRestartingVsCode] = useState(false);
  const [isSavingUnifyHistory, setIsSavingUnifyHistory] = useState(false);
  const [isMigratingUnifiedHistory, setIsMigratingUnifiedHistory] =
    useState(false);
  const [showUnifyEnableConfirm, setShowUnifyEnableConfirm] = useState(false);
  const [showUnifyDisableConfirm, setShowUnifyDisableConfirm] =
    useState(false);
  const [hasUnifyBackup, setHasUnifyBackup] = useState(false);
  const handleCodexQuickConfigured = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["providers", appId] }),
      queryClient.invalidateQueries({ queryKey: usageKeys.script("default", appId) }),
      queryClient.invalidateQueries({ queryKey: subscriptionKeys.quota(appId) }),
      queryClient.invalidateQueries({ queryKey: ["codex_oauth", "quota"] }),
    ]);
  }, [appId, queryClient]);

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


 return <div className="space-y-4 border-t border-border pt-4">
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

{codexQuickSetup}</div>;
}
