use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use toml_edit::{value, DocumentMut, Item, Table};
use uuid::Uuid;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(target_os = "windows"))]
fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    id: String,
    #[serde(default)]
    workspace_id: String,
    name: String,
    kind: ProfileKind,
    notes: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    config_toml: Option<String>,
    auth_json: Option<String>,
    #[serde(default)]
    codex_system: CodexSystem,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    ChatGptLogin,
    ProxyApiKey,
    Custom,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexSystem {
    #[default]
    Account,
    Api,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    active_profile_id: Option<String>,
    profiles: Vec<Profile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    id: String,
    workspace_id: String,
    name: String,
    kind: ProfileKind,
    notes: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    config_hash: Option<String>,
    auth_hash: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    account_email: Option<String>,
    account_name: Option<String>,
    account_plan: Option<String>,
    account_id: Option<String>,
    has_config: bool,
    has_auth: bool,
    codex_system: CodexSystem,
    is_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCodexState {
    codex_dir: String,
    config_path: String,
    auth_path: String,
    config_exists: bool,
    auth_exists: bool,
    config_hash: Option<String>,
    auth_hash: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    account_email: Option<String>,
    account_name: Option<String>,
    account_plan: Option<String>,
    account_id: Option<String>,
    auth_mode: String,
    active_profile_id: Option<String>,
    session_size: u64,
}

#[derive(Debug, Clone, Default)]
struct AccountInfo {
    email: Option<String>,
    name: Option<String>,
    plan: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountState {
    current: CurrentCodexState,
    profiles: Vec<ProfileSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchProfileResult {
    message: String,
    app_state: CodexAccountState,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthLoginResult {
    message: String,
    verification_url: Option<String>,
    user_code: Option<String>,
    expires_in_minutes: Option<u32>,
    output: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportInput {
    name: String,
    notes: Option<String>,
    kind: ProfileKind,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyProfileInput {
    name: String,
    base_url: String,
    api_key: String,
    model: String,
    review_model: String,
    reasoning_effort: String,
    notes: Option<String>,
    codex_system: Option<CodexSystem>,
}

fn codex_dir() -> PathBuf {
    crate::codex_config::get_codex_config_dir()
}

fn app_dir() -> PathBuf {
    crate::config::get_app_config_dir().join("codex-accounts")
}

fn store_path() -> PathBuf {
    app_dir().join("profiles.json")
}

fn managed_session_paths() -> &'static [&'static str] {
    &[
        "sessions",
        "archived_sessions",
        "session_index.jsonl",
        "history.jsonl",
        "state_5.sqlite",
        "state_5.sqlite-shm",
        "state_5.sqlite-wal",
        "goals_1.sqlite",
        "goals_1.sqlite-shm",
        "goals_1.sqlite-wal",
    ]
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|err| format!("读取 {} 失败：{}", path.to_string_lossy(), err))
}

fn write_optional(path: &Path, content: &Option<String>) -> Result<(), String> {
    match content {
        Some(value) => crate::config::write_text_file(path, value)
            .map_err(|err| format!("写入 {} 失败：{}", path.to_string_lossy(), err)),
        None => {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|err| format!("删除 {} 失败：{}", path.to_string_lossy(), err))?;
            }
            Ok(())
        }
    }
}

fn path_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| path_size(&entry.path()))
        .sum()
}

fn short_hash(content: &Option<String>) -> Option<String> {
    content.as_ref().map(|value| {
        let digest = Sha256::digest(value.as_bytes());
        format!("{:x}", digest)[..12].to_string()
    })
}

fn load_store() -> Result<Store, String> {
    let path = store_path();
    if !path.exists() {
        return Ok(Store {
            active_profile_id: None,
            profiles: vec![],
        });
    }
    let raw = fs::read_to_string(&path).map_err(|err| format!("读取 Codex 档案库失败：{}", err))?;
    let mut store: Store =
        serde_json::from_str(&raw).map_err(|err| format!("解析 Codex 档案库失败：{}", err))?;
    let mut changed = false;
    for profile in &mut store.profiles {
        if profile.workspace_id.is_empty() {
            profile.workspace_id = Uuid::new_v4().to_string();
            changed = true;
        }
        if profile.kind == ProfileKind::ProxyApiKey
            && profile.codex_system == CodexSystem::Account
            && auth_has_api_key(&profile.auth_json)
            && !auth_has_login_tokens(&profile.auth_json)
        {
            profile.codex_system = CodexSystem::Api;
            changed = true;
        }
        if profile.codex_system == CodexSystem::Api {
            let before = profile.config_toml.clone();
            ensure_api_profile_files(profile)?;
            if profile.config_toml != before {
                changed = true;
            }
        }
    }
    if changed {
        save_store(&store)?;
    }
    Ok(store)
}

fn save_store(store: &Store) -> Result<(), String> {
    let dir = app_dir();
    fs::create_dir_all(&dir).map_err(|err| format!("创建 Codex 档案目录失败：{}", err))?;
    let raw = serde_json::to_string_pretty(store)
        .map_err(|err| format!("序列化 Codex 档案库失败：{}", err))?;
    crate::config::write_text_file(&store_path(), &raw)
        .map_err(|err| format!("保存 Codex 档案库失败：{}", err))
}

fn extract_toml_value(raw: &Option<String>, key: &str) -> Option<String> {
    let raw = raw.as_ref()?;
    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return None;
        }
        let (left, right) = trimmed.split_once('=')?;
        if left.trim() != key {
            return None;
        }
        Some(right.trim().trim_matches('"').to_string())
    })
}

fn extract_base_url(raw: &Option<String>) -> Option<String> {
    let raw_config = raw.as_ref()?;
    let Ok(doc) = raw_config.parse::<DocumentMut>() else {
        return extract_toml_value(raw, "openai_base_url")
            .or_else(|| extract_toml_value(raw, "base_url"));
    };
    doc.get("openai_base_url")
        .and_then(Item::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            doc.get("model_provider")
                .and_then(Item::as_str)
                .and_then(|provider| {
                    doc.get("model_providers")
                        .and_then(Item::as_table_like)
                        .and_then(|providers| providers.get(provider))
                        .and_then(Item::as_table_like)
                        .and_then(|provider_table| provider_table.get("base_url"))
                        .and_then(Item::as_str)
                        .map(ToString::to_string)
                })
        })
        .or_else(|| extract_toml_value(raw, "base_url"))
}

fn auth_mode(auth: &Option<String>) -> String {
    let Some(raw) = auth else {
        return "未发现 auth.json".to_string();
    };
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value)
            if value
                .get("OPENAI_API_KEY")
                .and_then(|key| key.as_str())
                .is_some_and(|key| !key.trim().is_empty()) =>
        {
            "API Key".to_string()
        }
        Ok(value)
            if value
                .get("tokens")
                .and_then(|tokens| tokens.get("id_token"))
                .is_some()
                || value.get("refresh_token").is_some() =>
        {
            "ChatGPT 登录授权".to_string()
        }
        Ok(_) => "自定义授权文件".to_string(),
        Err(_) => "auth.json 格式异常".to_string(),
    }
}

fn string_at(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in keys {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str().map(ToString::to_string)
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn account_info(auth: &Option<String>) -> AccountInfo {
    let Some(raw) = auth else {
        return AccountInfo::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return AccountInfo::default();
    };

    let mut info = AccountInfo {
        account_id: string_at(&value, &["tokens", "account_id"]),
        ..AccountInfo::default()
    };

    if let Some(token) = string_at(&value, &["tokens", "id_token"]) {
        if let Some(payload) = decode_jwt_payload(&token) {
            info.email = string_at(&payload, &["email"]);
            info.name = string_at(&payload, &["name"]);
            info.plan = string_at(
                &payload,
                &["https://api.openai.com/auth", "chatgpt_plan_type"],
            );
            info.account_id = info.account_id.or_else(|| {
                string_at(
                    &payload,
                    &["https://api.openai.com/auth", "chatgpt_account_id"],
                )
            });
        }
    }

    info
}

fn auth_has_api_key(auth: &Option<String>) -> bool {
    let Some(raw) = auth else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    value
        .get("OPENAI_API_KEY")
        .and_then(|key| key.as_str())
        .is_some_and(|key| !key.trim().is_empty())
}

fn auth_has_login_tokens(auth: &Option<String>) -> bool {
    let Some(raw) = auth else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    value
        .get("tokens")
        .and_then(|tokens| tokens.get("id_token"))
        .is_some()
        || value.get("refresh_token").is_some()
}

fn active_profile<'a>(
    store: &'a Store,
    current_config: &Option<String>,
    current_auth: &Option<String>,
) -> Option<&'a Profile> {
    if let Some(active_id) = &store.active_profile_id {
        if let Some(profile) = store
            .profiles
            .iter()
            .find(|profile| &profile.id == active_id)
        {
            return Some(profile);
        }
    }
    let current_config_hash = short_hash(current_config);
    let current_auth_hash = short_hash(current_auth);
    store.profiles.iter().find(|profile| {
        short_hash(&profile.config_toml) == current_config_hash
            && short_hash(&profile.auth_json) == current_auth_hash
    })
}

fn current_session_size() -> u64 {
    let dir = codex_dir();
    managed_session_paths()
        .iter()
        .map(|relative| path_size(&dir.join(relative)))
        .sum()
}

fn current_files() -> Result<(Option<String>, Option<String>), String> {
    let dir = codex_dir();
    Ok((
        read_optional(&dir.join("config.toml"))?,
        read_optional(&dir.join("auth.json"))?,
    ))
}

fn current_state(active_profile_id: Option<String>) -> Result<CurrentCodexState, String> {
    let dir = codex_dir();
    let (config, auth) = current_files()?;
    let account = account_info(&auth);
    Ok(CurrentCodexState {
        codex_dir: dir.to_string_lossy().to_string(),
        config_path: dir.join("config.toml").to_string_lossy().to_string(),
        auth_path: dir.join("auth.json").to_string_lossy().to_string(),
        config_exists: config.is_some(),
        auth_exists: auth.is_some(),
        config_hash: short_hash(&config),
        auth_hash: short_hash(&auth),
        model: extract_toml_value(&config, "model"),
        base_url: extract_base_url(&config),
        account_email: account.email,
        account_name: account.name,
        account_plan: account.plan,
        account_id: account.account_id,
        auth_mode: auth_mode(&auth),
        active_profile_id,
        session_size: current_session_size(),
    })
}

fn summarize(
    profile: &Profile,
    current_config: &Option<String>,
    current_auth: &Option<String>,
    active_profile_id: Option<&str>,
) -> ProfileSummary {
    let profile_config_hash = short_hash(&profile.config_toml);
    let profile_auth_hash = short_hash(&profile.auth_json);
    let account = account_info(&profile.auth_json);
    ProfileSummary {
        id: profile.id.clone(),
        workspace_id: profile.workspace_id.clone(),
        name: profile.name.clone(),
        kind: profile.kind.clone(),
        notes: profile.notes.clone(),
        created_at: profile.created_at,
        updated_at: profile.updated_at,
        config_hash: profile_config_hash.clone(),
        auth_hash: profile_auth_hash.clone(),
        model: extract_toml_value(&profile.config_toml, "model"),
        base_url: extract_base_url(&profile.config_toml),
        account_email: account.email,
        account_name: account.name,
        account_plan: account.plan,
        account_id: account.account_id,
        has_config: profile.config_toml.is_some(),
        has_auth: profile.auth_json.is_some(),
        codex_system: profile.codex_system.clone(),
        is_active: active_profile_id == Some(profile.id.as_str())
            || (profile_config_hash == short_hash(current_config)
                && profile_auth_hash == short_hash(current_auth)),
    }
}

fn backup_current() -> Result<Option<String>, String> {
    let dir = codex_dir();
    let config_path = dir.join("config.toml");
    let auth_path = dir.join("auth.json");
    if !config_path.exists() && !auth_path.exists() {
        return Ok(None);
    }

    let stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let backup_dir = app_dir().join("backups").join(stamp);
    fs::create_dir_all(&backup_dir).map_err(|err| format!("创建备份目录失败：{}", err))?;
    if config_path.exists() {
        fs::copy(&config_path, backup_dir.join("config.toml"))
            .map_err(|err| format!("备份 config.toml 失败：{}", err))?;
    }
    if auth_path.exists() {
        fs::copy(&auth_path, backup_dir.join("auth.json"))
            .map_err(|err| format!("备份 auth.json 失败：{}", err))?;
    }
    Ok(Some(backup_dir.to_string_lossy().to_string()))
}

fn default_account_config_document() -> DocumentMut {
    r#"model_provider = "openai"
model = "gpt-5.5"
review_model = "gpt-5.5"
model_reasoning_effort = "xhigh"
disable_response_storage = true
network_access = "enabled"
windows_wsl_setup_acknowledged = true
model_context_window = 1000000
model_auto_compact_token_limit = 900000
"#
    .parse::<DocumentMut>()
    .unwrap_or_default()
}

fn account_mode_config(raw: Option<&str>) -> String {
    let mut doc = raw
        .and_then(|value| value.parse::<DocumentMut>().ok())
        .unwrap_or_else(default_account_config_document);

    doc["model_provider"] = value("openai");
    if !doc.contains_key("model") {
        doc["model"] = value("gpt-5.5");
    }
    if !doc.contains_key("review_model") {
        doc["review_model"] = value("gpt-5.5");
    }
    if !doc.contains_key("model_reasoning_effort") {
        doc["model_reasoning_effort"] = value("xhigh");
    }
    if !doc.contains_key("disable_response_storage") {
        doc["disable_response_storage"] = value(true);
    }
    if !doc.contains_key("network_access") {
        doc["network_access"] = value("enabled");
    }
    if !doc.contains_key("windows_wsl_setup_acknowledged") {
        doc["windows_wsl_setup_acknowledged"] = value(true);
    }
    if !doc.contains_key("model_context_window") {
        doc["model_context_window"] = value(1_000_000);
    }
    if !doc.contains_key("model_auto_compact_token_limit") {
        doc["model_auto_compact_token_limit"] = value(900_000);
    }

    doc.remove("openai_base_url");
    doc.remove("chatgpt_base_url");
    if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
        providers.remove("OpenAI");
        providers.remove("openai");
        if providers.is_empty() {
            doc.remove("model_providers");
        }
    }
    format!("{}\n", doc.to_string().trim_end())
}

fn proxy_base_config_document(
    model: &str,
    review_model: &str,
    effort: &str,
    provider: &str,
) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["model_provider"] = value(provider);
    doc["model"] = value(model);
    doc["review_model"] = value(review_model);
    doc["model_reasoning_effort"] = value(effort);
    doc["disable_response_storage"] = value(true);
    doc["network_access"] = value("enabled");
    doc["windows_wsl_setup_acknowledged"] = value(true);
    doc["model_context_window"] = value(1_000_000);
    doc["model_auto_compact_token_limit"] = value(900_000);
    doc
}

fn trim_api_version_suffix(raw: &str) -> String {
    let value = raw.trim().trim_end_matches('/');
    value
        .strip_suffix("/v1")
        .unwrap_or(value)
        .trim_end_matches('/')
        .to_string()
}

fn normalize_proxy_base_url(raw: &str) -> String {
    trim_api_version_suffix(raw)
}

fn api_provider_base_url(raw: &str) -> String {
    let base_url = normalize_proxy_base_url(raw);
    if base_url.ends_with("/v1") {
        base_url
    } else {
        format!("{base_url}/v1")
    }
}

fn proxy_account_config(model: &str, review_model: &str, effort: &str, base_url: &str) -> String {
    let mut doc = proxy_base_config_document(model, review_model, effort, "openai");
    doc["openai_base_url"] = value(normalize_proxy_base_url(base_url));
    format!("{}\n", doc.to_string().trim_end())
}

fn proxy_api_config(model: &str, review_model: &str, effort: &str, base_url: &str) -> String {
    let mut doc = proxy_base_config_document(model, review_model, effort, "OpenAI");
    let mut providers = Table::new();
    providers.set_implicit(true);
    let mut openai = Table::new();
    openai["name"] = value("OpenAI");
    openai["base_url"] = value(api_provider_base_url(base_url));
    openai["wire_api"] = value("responses");
    openai["requires_openai_auth"] = value(true);
    providers["OpenAI"] = Item::Table(openai);
    doc["model_providers"] = Item::Table(providers);
    format!("{}\n", doc.to_string().trim_end())
}

fn api_auth_json(api_key: &str) -> String {
    serde_json::json!({ "OPENAI_API_KEY": api_key.trim() }).to_string()
}

fn ensure_api_profile_files(profile: &mut Profile) -> Result<(), String> {
    if profile.codex_system != CodexSystem::Api {
        return Ok(());
    }

    if !auth_has_api_key(&profile.auth_json) {
        return Err(format!(
            "API Key 档案「{}」没有写入 OPENAI_API_KEY，请重新填写 Key 后创建。",
            profile.name
        ));
    }

    let config = profile.config_toml.clone().ok_or_else(|| {
        format!(
            "API Key 档案「{}」缺少 config.toml，请重新创建。",
            profile.name
        )
    })?;
    let model =
        extract_toml_value(&Some(config.clone()), "model").unwrap_or_else(|| "gpt-5.5".to_string());
    let review_model =
        extract_toml_value(&Some(config.clone()), "review_model").unwrap_or_else(|| model.clone());
    let effort = extract_toml_value(&Some(config.clone()), "model_reasoning_effort")
        .unwrap_or_else(|| "xhigh".to_string());
    let base_url = extract_base_url(&Some(config)).ok_or_else(|| {
        format!(
            "API Key 档案「{}」缺少 Base URL，请重新创建。",
            profile.name
        )
    })?;
    let base_url = trim_api_version_suffix(&base_url);

    profile.config_toml = Some(proxy_api_config(&model, &review_model, &effort, &base_url));
    Ok(())
}

fn get_state_impl() -> Result<CodexAccountState, String> {
    let (current_config, current_auth) = current_files()?;
    let store = load_store()?;
    let active_id =
        active_profile(&store, &current_config, &current_auth).map(|profile| profile.id.clone());
    let current = current_state(active_id.clone())?;
    let profiles = store
        .profiles
        .iter()
        .map(|profile| {
            summarize(
                profile,
                &current_config,
                &current_auth,
                active_id.as_deref(),
            )
        })
        .collect();

    Ok(CodexAccountState { current, profiles })
}

#[tauri::command]
pub async fn codex_account_state() -> Result<CodexAccountState, String> {
    tauri::async_runtime::spawn_blocking(get_state_impl)
        .await
        .map_err(|err| format!("读取 Codex 账号状态任务异常退出：{err}"))?
}

#[tauri::command]
pub async fn codex_account_import_current_profile(
    input: ImportInput,
) -> Result<CodexAccountState, String> {
    tauri::async_runtime::spawn_blocking(move || import_current_profile_impl(input))
        .await
        .map_err(|err| format!("导入 Codex 档案任务异常退出：{err}"))?
}

fn import_current_profile_impl(input: ImportInput) -> Result<CodexAccountState, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("请输入档案名称".to_string());
    }

    let (config_toml, auth_json) = current_files()?;
    if config_toml.is_none() && auth_json.is_none() {
        return Err("当前 Codex 目录下没有可导入的 config.toml 或 auth.json".to_string());
    }

    let codex_system = if auth_has_api_key(&auth_json) && !auth_has_login_tokens(&auth_json) {
        CodexSystem::Api
    } else {
        CodexSystem::Account
    };
    let config_toml = match codex_system {
        CodexSystem::Account => config_toml.map(|raw| account_mode_config(Some(&raw))),
        CodexSystem::Api => config_toml,
    };
    let now = Utc::now();
    let mut store = load_store()?;
    store.profiles.push(Profile {
        id: Uuid::new_v4().to_string(),
        workspace_id: Uuid::new_v4().to_string(),
        codex_system,
        name: name.to_string(),
        kind: input.kind,
        notes: input.notes.unwrap_or_default(),
        created_at: now,
        updated_at: now,
        config_toml,
        auth_json,
    });
    save_store(&store)?;
    get_state_impl()
}

#[tauri::command]
pub async fn codex_account_create_proxy_profile(
    input: ProxyProfileInput,
) -> Result<CodexAccountState, String> {
    tauri::async_runtime::spawn_blocking(move || create_proxy_profile_impl(input))
        .await
        .map_err(|err| format!("创建 Codex API Key 环境任务异常退出：{err}"))?
}

fn create_proxy_profile_impl(input: ProxyProfileInput) -> Result<CodexAccountState, String> {
    let name = input.name.trim();
    let base_url = normalize_proxy_base_url(&input.base_url);
    let api_key = input.api_key.trim();
    let codex_system = input.codex_system.unwrap_or(CodexSystem::Api);
    if name.is_empty() || base_url.is_empty() {
        return Err("名称和 Base URL 不能为空".to_string());
    }
    if codex_system == CodexSystem::Api && api_key.is_empty() {
        return Err("API Key 环境需要填写 API Key".to_string());
    }

    let model = if input.model.trim().is_empty() {
        "gpt-5.5"
    } else {
        input.model.trim()
    };
    let review_model = if input.review_model.trim().is_empty() {
        model
    } else {
        input.review_model.trim()
    };
    let effort = if input.reasoning_effort.trim().is_empty() {
        "xhigh"
    } else {
        input.reasoning_effort.trim()
    };

    let config_toml = match codex_system {
        CodexSystem::Account => proxy_account_config(model, review_model, effort, &base_url),
        CodexSystem::Api => proxy_api_config(model, review_model, effort, &base_url),
    };
    let auth_json = match codex_system {
        CodexSystem::Account => {
            let (_, current_auth) = current_files()?;
            if auth_mode(&current_auth) == "ChatGPT 登录授权" {
                current_auth.unwrap_or_else(|| api_auth_json(api_key))
            } else if api_key.is_empty() {
                return Err(
                    "当前没有 ChatGPT 登录授权；请先设备码登录，或填写 API Key。".to_string(),
                );
            } else {
                api_auth_json(api_key)
            }
        }
        CodexSystem::Api => api_auth_json(api_key),
    };

    let now = Utc::now();
    let mut store = load_store()?;
    store.profiles.push(Profile {
        id: Uuid::new_v4().to_string(),
        workspace_id: Uuid::new_v4().to_string(),
        codex_system,
        name: name.to_string(),
        kind: ProfileKind::ProxyApiKey,
        notes: input.notes.unwrap_or_default(),
        created_at: now,
        updated_at: now,
        config_toml: Some(config_toml),
        auth_json: Some(format!("{}\n", auth_json.trim_end())),
    });
    save_store(&store)?;
    get_state_impl()
}

#[tauri::command]
pub async fn codex_account_switch_profile(id: String) -> Result<SwitchProfileResult, String> {
    tauri::async_runtime::spawn_blocking(move || switch_profile_impl(id))
        .await
        .map_err(|err| format!("切换 Codex 档案任务异常退出：{err}"))?
}

fn switch_profile_impl(id: String) -> Result<SwitchProfileResult, String> {
    let mut store = load_store()?;
    let target_profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == id)
        .cloned()
        .ok_or_else(|| "找不到指定档案".to_string())?;

    let dir = codex_dir();
    fs::create_dir_all(&dir).map_err(|err| format!("创建 Codex 目录失败：{}", err))?;
    backup_current()?;
    write_optional(&dir.join("config.toml"), &target_profile.config_toml)?;
    write_optional(&dir.join("auth.json"), &target_profile.auth_json)?;
    store.active_profile_id = Some(target_profile.id);
    save_store(&store)?;
    Ok(SwitchProfileResult {
        message: "已切换 Codex 档案；正在使用的 Codex App/CLI 可能需要重启后完全生效。".to_string(),
        app_state: get_state_impl()?,
    })
}

#[tauri::command]
pub async fn codex_account_delete_profile(id: String) -> Result<CodexAccountState, String> {
    tauri::async_runtime::spawn_blocking(move || delete_profile_impl(id))
        .await
        .map_err(|err| format!("删除 Codex 档案任务异常退出：{err}"))?
}

fn delete_profile_impl(id: String) -> Result<CodexAccountState, String> {
    let mut store = load_store()?;
    let before = store.profiles.len();
    store.profiles.retain(|profile| profile.id != id);
    if store.profiles.len() == before {
        return Err("找不到指定档案".to_string());
    }
    if store.active_profile_id.as_deref() == Some(id.as_str()) {
        store.active_profile_id = None;
    }
    save_store(&store)?;
    get_state_impl()
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        output.push(ch);
    }
    output
}

fn extract_device_auth_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|part| {
            part.trim_matches(|ch: char| {
                ch == '"' || ch == '\'' || ch == ')' || ch == '(' || ch == ',' || ch == '.'
            })
        })
        .find(|part| part.starts_with("http") && part.contains("/codex/device"))
        .map(ToString::to_string)
}

fn extract_device_auth_code(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|part| {
        let cleaned = part
            .trim_matches(|ch: char| {
                ch == '"' || ch == '\'' || ch == ')' || ch == '(' || ch == ',' || ch == '.'
            })
            .to_string();
        let sections = cleaned.split('-').collect::<Vec<_>>();
        let looks_like_code = sections.len() == 2
            && sections.iter().all(|section| {
                !section.is_empty() && section.chars().all(|ch| ch.is_ascii_alphanumeric())
            });
        looks_like_code.then_some(cleaned)
    })
}

fn spawn_codex_device_auth_child() -> Result<std::process::Child, String> {
    let mut errors = Vec::new();

    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &["codex.cmd", "codex"];
    #[cfg(not(target_os = "windows"))]
    let candidates: &[&str] = &["codex"];

    for program in candidates {
        match hidden_command(program)
            .args(["login", "--device-auth"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(err) => errors.push(format!("{program}: {err}")),
        }
    }

    Err(format!(
        "启动 codex login --device-auth 失败：{}",
        errors.join("；")
    ))
}

fn start_device_auth_login_impl() -> Result<DeviceAuthLoginResult, String> {
    let mut child = spawn_codex_device_auth_child()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 codex device auth 输出".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取 codex device auth 错误输出".to_string())?;
    let (tx, rx) = mpsc::channel::<String>();
    let tx_stdout = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx_stdout.send(line);
        }
    });
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    let started_at = Instant::now();
    let timeout = Duration::from_secs(8);
    let mut output_lines = Vec::new();
    let mut verification_url = None;
    let mut user_code = None;
    while started_at.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                let cleaned = strip_ansi_sequences(&line);
                if !cleaned.trim().is_empty() {
                    output_lines.push(cleaned);
                }
                let joined = output_lines.join("\n");
                verification_url = extract_device_auth_url(&joined);
                user_code = extract_device_auth_code(&joined);
                if verification_url.is_some() && user_code.is_some() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(status)) = child.try_wait() {
                    let joined = output_lines.join("\n");
                    return Err(format!(
                        "codex login --device-auth 提前退出：{status}。输出：{}",
                        joined.trim()
                    ));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let output = output_lines.join("\n");
    if verification_url.is_none() || user_code.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "已启动 codex login --device-auth，但未在 {timeout:?} 内读取到设备码。输出：{}",
            output.trim()
        ));
    }

    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(DeviceAuthLoginResult {
        message: "已启动 Codex 设备码登录。请在浏览器打开链接并输入一次性 code；授权完成后刷新状态或导入当前档案。".to_string(),
        verification_url,
        user_code,
        expires_in_minutes: Some(15),
        output,
    })
}

#[tauri::command]
pub async fn codex_account_start_device_auth_login() -> Result<DeviceAuthLoginResult, String> {
    tauri::async_runtime::spawn_blocking(start_device_auth_login_impl)
        .await
        .map_err(|err| format!("设备码登录任务异常退出：{err}"))?
}

#[tauri::command]
pub async fn codex_account_open_file(app: AppHandle, name: String) -> Result<String, String> {
    let allowed = match name.as_str() {
        "config.toml" | "auth.json" => name,
        _ => return Err("只能打开 Codex 的 config.toml 或 auth.json".to_string()),
    };
    let path = codex_dir().join(&allowed);
    if !path.exists() {
        return Err(format!("文件不存在：{}", path.to_string_lossy()));
    }
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|err| format!("打开 {} 失败：{}", allowed, err))?;
    Ok(path.to_string_lossy().to_string())
}
