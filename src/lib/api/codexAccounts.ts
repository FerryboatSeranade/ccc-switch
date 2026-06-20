import { invoke } from "@tauri-apps/api/core";

export type CodexProfileKind = "chat_gpt_login" | "proxy_api_key" | "custom";
export type CodexSystem = "account" | "api";

export interface CodexProfileSummary {
  id: string;
  workspaceId: string;
  name: string;
  kind: CodexProfileKind;
  notes: string;
  createdAt: string;
  updatedAt: string;
  configHash: string | null;
  authHash: string | null;
  model: string | null;
  baseUrl: string | null;
  accountEmail: string | null;
  accountName: string | null;
  accountPlan: string | null;
  accountId: string | null;
  hasConfig: boolean;
  hasAuth: boolean;
  codexSystem: CodexSystem;
  isActive: boolean;
}

export interface CurrentCodexState {
  codexDir: string;
  configPath: string;
  authPath: string;
  configExists: boolean;
  authExists: boolean;
  configHash: string | null;
  authHash: string | null;
  model: string | null;
  baseUrl: string | null;
  accountEmail: string | null;
  accountName: string | null;
  accountPlan: string | null;
  accountId: string | null;
  authMode: string;
  activeProfileId: string | null;
  sessionSize: number;
}

export interface CodexAccountState {
  current: CurrentCodexState;
  profiles: CodexProfileSummary[];
}

export interface DeviceAuthLoginResult {
  message: string;
  verificationUrl: string | null;
  userCode: string | null;
  expiresInMinutes: number | null;
  output: string;
}

export interface SwitchProfileResult {
  message: string;
  appState: CodexAccountState;
}

export interface ImportCurrentProfileInput {
  name: string;
  notes?: string | null;
  kind: CodexProfileKind;
}

export interface CreateProxyProfileInput {
  name: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  reviewModel: string;
  reasoningEffort: string;
  notes?: string | null;
  codexSystem?: CodexSystem | null;
}

export const codexAccountsApi = {
  async getState(): Promise<CodexAccountState> {
    return await invoke("codex_account_state");
  },

  async importCurrentProfile(
    input: ImportCurrentProfileInput,
  ): Promise<CodexAccountState> {
    return await invoke("codex_account_import_current_profile", { input });
  },

  async createProxyProfile(
    input: CreateProxyProfileInput,
  ): Promise<CodexAccountState> {
    return await invoke("codex_account_create_proxy_profile", { input });
  },

  async switchProfile(id: string): Promise<SwitchProfileResult> {
    return await invoke("codex_account_switch_profile", { id });
  },

  async deleteProfile(id: string): Promise<CodexAccountState> {
    return await invoke("codex_account_delete_profile", { id });
  },

  async startDeviceAuthLogin(): Promise<DeviceAuthLoginResult> {
    return await invoke("codex_account_start_device_auth_login");
  },

  async openFile(name: "config.toml" | "auth.json"): Promise<string> {
    return await invoke("codex_account_open_file", { name });
  },
};
