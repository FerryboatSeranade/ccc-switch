import { getVersion } from "@tauri-apps/api/app";
import { isUpdateAvailable } from "./version";

export type UpdateChannel = "stable" | "beta";
export type UpdateSource = "tauri-updater" | "github-release";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
  source?: UpdateSource;
  downloadUrl?: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForUpdate(
  opts: CheckOptions = {},
): Promise<
  { status: "up-to-date" } | { status: "available"; info: UpdateInfo }
> {
  const currentVersion = await getCurrentVersion();
  let tauriCheckSucceeded = false;

  try {
    // 动态引入，避免在未安装插件时导致打包期问题
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check({ timeout: opts.timeout ?? 30000 } as any);
    tauriCheckSucceeded = true;

    if (update) {
      const info: UpdateInfo = {
        currentVersion,
        availableVersion: (update as any).version ?? "",
        notes: (update as any).notes,
        pubDate: (update as any).date,
        source: "tauri-updater",
      };

      return { status: "available", info };
    }
  } catch (error) {
    console.warn("[updater] Tauri updater check failed, trying GitHub", error);
  }

  try {
    const githubResult = await checkGithubRelease(currentVersion);
    if (githubResult) {
      return { status: "available", info: githubResult };
    }
  } catch (error) {
    if (!tauriCheckSucceeded) {
      throw error;
    }
    console.warn("[updater] GitHub fallback check failed", error);
  }

  return { status: "up-to-date" };
}

async function checkGithubRelease(
  currentVersion: string,
): Promise<UpdateInfo | null> {
  if (!currentVersion) return null;

  const response = await fetch(
    "https://api.github.com/repos/FerryboatSeranade/codex-switch/releases/latest",
    {
      headers: {
        Accept: "application/vnd.github+json",
      },
    },
  );

  if (!response.ok) {
    throw new Error(`GitHub release check failed: HTTP ${response.status}`);
  }

  const release = (await response.json()) as {
    tag_name?: string;
    body?: string;
    html_url?: string;
    published_at?: string;
  };
  const latestVersion = (release.tag_name ?? "").trim().replace(/^v/i, "");

  if (!isUpdateAvailable(currentVersion, latestVersion)) {
    return null;
  }

  return {
    currentVersion,
    availableVersion: latestVersion,
    notes: release.body,
    pubDate: release.published_at,
    source: "github-release",
    downloadUrl: release.html_url,
  };
}
