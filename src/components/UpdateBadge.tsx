import { useUpdate } from "@/contexts/UpdateContext";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ArrowUpCircle, Loader2, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";

interface UpdateBadgeProps {
  className?: string;
  onClick?: () => void;
  alwaysVisible?: boolean;
  isInstalling?: boolean;
}

export function UpdateBadge({
  className = "",
  onClick,
  alwaysVisible = false,
  isInstalling = false,
}: UpdateBadgeProps) {
  const { hasUpdate, updateInfo, isChecking } = useUpdate();
  const { t } = useTranslation();
  const isActive = hasUpdate && updateInfo;
  const isBusy = isInstalling || isChecking;
  const title = isInstalling
    ? t("settings.updating")
    : isActive
      ? t("settings.updateAvailable", {
          version: updateInfo?.availableVersion ?? "",
        })
      : isChecking
        ? t("settings.checking")
        : t("settings.checkForUpdates");

  if (!alwaysVisible && !isActive) {
    return null;
  }

  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            title={title}
            aria-label={title}
            aria-disabled={isBusy}
            aria-busy={isBusy}
            onClick={isBusy ? undefined : onClick}
            className={cn(
              "relative h-8 w-8 rounded-full",
              isActive
                ? "text-green-600 hover:bg-green-50 dark:text-green-400 dark:hover:bg-green-500/10"
                : "text-muted-foreground hover:bg-muted/60",
              isBusy && "cursor-default opacity-80",
              className,
            )}
          >
            {isInstalling ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : isActive ? (
              <ArrowUpCircle className="h-5 w-5" />
            ) : (
              <RefreshCw
                className={cn("h-4 w-4", isChecking && "animate-spin")}
              />
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">{title}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
