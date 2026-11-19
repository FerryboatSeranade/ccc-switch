#![allow(non_snake_case)]
#[cfg(target_os = "windows")]
use encoding_rs::GBK;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use crate::error::AppError;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// 应用更新下载进度（通过 `update-download-progress` 事件发给前端）。
#[derive(Clone, serde::Serialize)]
struct UpdateDownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 开启统一会话历史后，后台把官方会话迁进共享组的那一轮跑完了（或失败）。
const CODEX_UNIFY_MIGRATION_FINISHED_EVENT: &str = "codex-unify-migration-finished";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexUnifyMigrationFinished {
    migrated_state_rows: usize,
    migrated_jsonl_files: usize,
    error: Option<String>,
}

fn merge_settings_for_save(
    mut incoming: crate::settings::AppSettings,
    existing: &crate::settings::AppSettings,
) -> crate::settings::AppSettings {
    match (&mut incoming.webdav_sync, &existing.webdav_sync) {
        // incoming 没有 webdav → 保留现有
        (None, _) => {
            incoming.webdav_sync = existing.webdav_sync.clone();
        }
        // incoming 有 webdav 但密码为空，且现有有密码 → 填回现有密码
        // （get_settings_for_frontend 总是清空密码，所以通过 save_settings
        //   传入的空密码意味着"保持现有"而非"用户主动清空"）
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.password.is_empty() && !existing_sync.password.is_empty() =>
        {
            incoming_sync.password = existing_sync.password.clone();
        }
        _ => {}
    }
    match (&mut incoming.s3_sync, &existing.s3_sync) {
        // incoming 没有 s3 → 保留现有
        (None, _) => {
            incoming.s3_sync = existing.s3_sync.clone();
        }
        // incoming 有 s3 但密钥为空，且现有有密钥 → 填回现有密钥
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.secret_access_key.is_empty()
                && !existing_sync.secret_access_key.is_empty() =>
        {
            incoming_sync.secret_access_key = existing_sync.secret_access_key.clone();
        }
        _ => {}
    }
    // local_migrations 是纯后端状态（迁移完成标记），前端没有合法的修改场景，
    // 无条件取现有值。若按 incoming 透传：后端清掉 marker（如关闭统一会话
    // 开关）后、前端 query 缓存刷新前的一次全量保存会把旧 marker 重放回来，
    // 重新开启时被"复活"的标记挡住而漏迁。
    incoming.local_migrations = existing.local_migrations.clone();
    // 客户端地址只经 set_proxy_client_host 保存（它还要重写客户端）；全量保存带的是前端
    // 缓存里的旧值，透传会把刚保存的地址冲掉。
    incoming.proxy_client_host = existing.proxy_client_host.clone();
    // 轻量模式状态只由进入 / 退出轻量模式时写，前端缓存里的值总是旧的。
    incoming.lightweight_mode = existing.lightweight_mode;
    incoming
}

#[tauri::command]
pub async fn get_settings() -> Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::get_settings_for_frontend())
}

#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::store::AppState>,
    settings: crate::settings::AppSettings,
) -> Result<bool, String> {
    let existing = crate::settings::get_settings();
    let merged = merge_settings_for_save(settings, &existing);
    let unify_codex_changed =
        merged.unify_codex_session_history != existing.unify_codex_session_history;
    let unify_codex_enabled = merged.unify_codex_session_history;
    // 经典子 agent 开关、聚合子 agent 模型/强度和聚合自动审批模型都只影响 Stack 模式下
    // 写给 Codex 的内容。
    let stack_settings_changed = merged.codex_stack_classic_tools
        != existing.codex_stack_classic_tools
        || merged.codex_stack_subagent_model != existing.codex_stack_subagent_model
        || merged.codex_stack_subagent_reasoning_effort
            != existing.codex_stack_subagent_reasoning_effort
        || merged.codex_stack_auto_review_model != existing.codex_stack_auto_review_model;
    let tray_style_changed = merged.native_tray_menu != existing.native_tray_menu;
    let without_login_changed =
        merged.codex_proxy_without_official_login != existing.codex_proxy_without_official_login;
    let preserve_login_changed = merged.preserve_codex_official_auth_on_switch
        != existing.preserve_codex_official_auth_on_switch;
    crate::settings::update_settings(merged)?;

    // 原生托盘菜单 / 托盘面板当场切换，不用重启。
    if tray_style_changed {
        crate::tray::apply_tray_style(&app);
    }

    // 立即重写，失败时回滚设置，免得界面显示已改好、客户端还是旧的。
    if stack_settings_changed {
        if let Err(err) = crate::mode::controller::resync_codex_stack_catalog(state.inner()).await {
            log::warn!("Codex 聚合设置变更后重写客户端配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.codex_stack_classic_tools = existing.codex_stack_classic_tools;
            rollback.codex_stack_subagent_model = existing.codex_stack_subagent_model.clone();
            rollback.codex_stack_subagent_reasoning_effort =
                existing.codex_stack_subagent_reasoning_effort.clone();
            rollback.codex_stack_auto_review_model = existing.codex_stack_auto_review_model.clone();
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚 Codex 聚合设置失败: {rollback_err}");
            }
            return Err(format!(
                "聚合设置未生效（Codex 配置重写失败） (The aggregation setting did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 「暂停使用官方登录」只影响路由 / 聚合模式下写给 Codex 的内容：立即重写，失败回滚。
    if without_login_changed {
        if let Err(err) = crate::mode::controller::resync_codex_proxy(state.inner()).await {
            log::warn!("「暂停使用官方登录」变更后重写 Codex 配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.codex_proxy_without_official_login =
                existing.codex_proxy_without_official_login;
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚「暂停使用官方登录」设置失败: {rollback_err}");
            }
            return Err(format!(
                "「暂停使用官方登录」未生效（Codex 配置重写失败） (\"Pause the official login\" did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 「非接管切换时保留官方登录」只影响直连切到第三方：当前就是直连第三方时立即重写，
    // 不必先切回官方卡再切回来；失败回滚。
    if preserve_login_changed {
        if let Err(err) =
            crate::services::provider::reapply_current_codex_third_party_live(state.inner())
        {
            log::warn!("「非接管切换时保留官方登录」变更后重写 Codex 配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.preserve_codex_official_auth_on_switch =
                existing.preserve_codex_official_auth_on_switch;
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚「非接管切换时保留官方登录」设置失败: {rollback_err}");
            }
            return Err(format!(
                "「非接管切换时保留官方登录」未生效（Codex 配置重写失败） (\"Keep official login for direct switches\" did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 统一会话开关变更时立即重写当前官方 Codex 供应商的 live 配置，
    // 不必等下一次切换才生效。
    if unify_codex_changed {
        // live 重写失败时回滚设置并把保存整体报失败：若设置保持已切换状态，
        // live 仍跑旧桶，后续的历史迁移/还原会让会话再次分裂（开启=历史
        // 迁走而新会话仍写 openai 桶；关闭=会话还原而 live 仍写 custom）。
        // 报错让前端 saved=false 短路还原；回滚是整次保存的事务语义
        // （本开关的保存只携带开关相关字段）。
        if let Err(err) =
            crate::services::provider::reapply_current_codex_official_live(state.inner())
        {
            log::warn!("统一 Codex 会话历史开关变更后重写 live 配置失败，回滚设置: {err}");
            if let Err(rollback_err) = crate::settings::update_settings(existing) {
                log::error!("回滚统一会话开关设置失败: {rollback_err}");
            }
            return Err(format!(
                "统一 Codex 会话历史开关未生效（live 配置重写失败）: {err}"
            ));
        }

        if unify_codex_enabled {
            // 后台把留在 openai 桶的会话迁进 custom 桶（仅当用户勾选了迁入既有
            // 会话，函数内部自门控）。大会话目录可能要读数秒，不能阻塞设置保存；
            // 失败时下次启动自动重试。
            // 迁完（或失败）发事件给前端提示：「开启统一」时前端只说了“后台迁入中”，
            // 用户要等到这一刻再重启 Codex，旧会话才会解锁。
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let finished = match crate::codex_history_migration::maybe_migrate_codex_official_history_to_unified_bucket() {
                    Ok(outcome) => {
                        if let Some(reason) = outcome.skipped_reason {
                            log::debug!("○ Codex official history unify migration skipped: {reason}");
                            None
                        } else {
                            log::info!(
                                "✓ Codex official history unify migration completed: jsonl_files={}, state_rows={}",
                                outcome.migrated_jsonl_files,
                                outcome.migrated_state_rows
                            );
                            Some(CodexUnifyMigrationFinished {
                                migrated_state_rows: outcome.migrated_state_rows,
                                migrated_jsonl_files: outcome.migrated_jsonl_files,
                                error: None,
                            })
                        }
                    }
                    Err(e) => {
                        log::warn!("✗ Codex official history unify migration failed: {e}");
                        Some(CodexUnifyMigrationFinished {
                            migrated_state_rows: 0,
                            migrated_jsonl_files: 0,
                            error: Some(e.to_string()),
                        })
                    }
                };
                if let Some(payload) = finished {
                    if let Err(e) = app.emit(CODEX_UNIFY_MIGRATION_FINISHED_EVENT, payload) {
                        log::warn!("发送 Codex 统一迁移完成事件失败: {e}");
                    }
                }
            });
        } else {
            // 清除标记与迁移意愿，让重新开启并再次勾选时能补迁
            // 关闭期间落入 openai 桶的官方会话。
            if let Err(err) = crate::settings::clear_codex_official_history_unify_migration() {
                log::warn!("清除统一会话迁移标记失败: {err}");
            }
            if let Err(err) = crate::settings::clear_codex_unify_migrate_existing() {
                log::warn!("清除统一会话迁移意愿失败: {err}");
            }
        }
    }
    Ok(true)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifyHistoryRestoreResult {
    pub restored_jsonl_files: usize,
    pub restored_state_rows: usize,
    /// 还原被跳过的原因（如当前目录没有账本），前端据此提示而非报"成功 0 项"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// 是否存在统一会话开关的迁移备份（决定关闭弹窗里是否显示"恢复备份"勾选）。
#[tauri::command]
pub async fn has_codex_unify_history_backup() -> Result<bool, String> {
    Ok(crate::codex_history_migration::has_codex_official_history_unify_backup())
}

/// Codex 的 `config.toml` 是不是用 `[features] multi_agent_v2` 强制了新版子 agent 工具
/// （这时「经典子 agent 工具」开关不生效，界面上提示）。
#[tauri::command]
pub async fn codex_forces_multi_agent_v2() -> Result<bool, String> {
    let text = crate::codex_config::read_codex_config_text()?;
    Ok(crate::codex_config::codex_config_forces_multi_agent_v2(
        &text,
    ))
}

/// Codex 会话压缩开关（`[features] local_thread_store_compression`）当前是否开启。
#[tauri::command]
pub async fn get_codex_session_compression() -> Result<bool, AppError> {
    crate::services::codex_session_compression::is_enabled()
}

/// 开关 Codex 会话压缩，返回写入后的状态。
#[tauri::command]
pub async fn set_codex_session_compression(
    state: tauri::State<'_, crate::store::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let db = state.db.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || {
        crate::services::codex_session_compression::set_enabled(&db, enabled)
    })
    .await
    .map_err(|e| e.to_string())??)
}

/// Codex 会话目录（sessions + archived_sessions）当前占用的字节数。
#[tauri::command]
pub async fn get_codex_sessions_disk_usage() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(
        crate::services::codex_session_compression::sessions_disk_usage,
    )
    .await
    .map_err(|e| e.to_string())
}

/// 按迁移备份账本把当时迁入共享桶的官方会话还原回 "openai" 桶。
/// 由关闭统一会话开关的确认弹窗触发；幂等，可安全重试。
#[tauri::command]
pub async fn restore_codex_unified_history() -> Result<CodexUnifyHistoryRestoreResult, String> {
    let outcome = tauri::async_runtime::spawn_blocking(|| {
        crate::codex_history_migration::restore_codex_official_history_from_backups()
    })
    .await
    .map_err(|e| e.to_string())??;

    if let Some(reason) = &outcome.skipped_reason {
        log::debug!("○ Codex official history restore skipped: {reason}");
    } else {
        log::info!(
            "✓ Codex official history restored from backups: jsonl_files={}, state_rows={}",
            outcome.restored_jsonl_files,
            outcome.restored_state_rows
        );
    }

    Ok(CodexUnifyHistoryRestoreResult {
        restored_jsonl_files: outcome.restored_jsonl_files,
        restored_state_rows: outcome.restored_state_rows,
        skipped_reason: outcome.skipped_reason,
    })
}

/// 重启应用程序（当 app_config_dir 变更后使用）
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<bool, String> {
    crate::save_window_state_before_exit(&app);

    // 在后台延迟重启，让函数有时间返回响应
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        // 本命令用于 app_config_dir 变更后的重启：新实例会切到新数据库，拿不到
        // 旧库里的 Live 备份，无法恢复被接管的 Live 配置。因此必须趁旧实例的
        // 事件循环仍存活，在这里同步完成恢复（保留代理状态，新实例启动时自动
        // 重新接管）。
        crate::cleanup_before_exit(&app).await;
        // 与自更新共用 restart_process，而不是 app.restart()：macOS 上经
        // LaunchServices 启动，新窗口才能回到前台。
        crate::restart_process(&app);
    });
    Ok(true)
}

/// 下载并安装应用更新，然后由后端直接重启应用。
///
/// 自动更新已经在后台下好同一版本时直接用缓存的包（见 `app_update`）；检查更新失败
/// （离线）但手里有下好的包，也装那个包。
#[tauri::command]
pub async fn install_update_and_restart(app: AppHandle) -> Result<bool, String> {
    // 弹窗、关于页、托盘可能先后点了安装：第二次直接拒绝，避免两次安装同时替换应用、做退出清理
    let Some(_installing) = crate::app_update::begin_install() else {
        return Err("正在安装更新，请稍候".to_string());
    };
    let checked = match crate::app_update::build_updater(&app)?.check().await {
        Ok(update) => update,
        Err(e) => {
            if let Some((update, bytes)) = crate::app_update::any_pending() {
                log::warn!("检查更新失败（{e}），改装已下载的 {}", update.version);
                return install_and_restart(app, update, bytes).await;
            }
            return Err(format!("检查更新失败: {e}"));
        }
    };
    let Some(update) = checked else {
        // 已是最新（下好的版本被撤回了）：缓存的包不能再装，提示和托盘入口一起收起
        crate::app_update::discard_pending(&app);
        return Ok(false);
    };

    let bytes = match crate::app_update::pending_bytes_for(&update.version) {
        Some(bytes) => {
            log::info!("使用已下载的应用更新: {}", update.version);
            bytes
        }
        None => {
            log::info!("开始下载应用更新: {}", update.version);
            let progress_handle = app.clone();
            let mut downloaded: u64 = 0;
            update
                .download(
                    move |chunk_len, content_len| {
                        downloaded = downloaded.saturating_add(chunk_len as u64);
                        let _ = progress_handle.emit(
                            "update-download-progress",
                            UpdateDownloadProgress {
                                downloaded,
                                total: content_len,
                            },
                        );
                    },
                    || {},
                )
                .await
                .map_err(|e| format!("下载更新失败: {e}"))?
        }
    };

    install_and_restart(app, update, bytes).await
}

/// macOS 更新会原地替换 `.app` bundle。如果先返回前端、再让旧 WebView 调
/// `process.relaunch()`，旧进程可能已经处在 bundle 被替换后的不稳定窗口期。
/// 这里把退出清理、安装和重启串在同一个后端流程中，避免依赖旧前端继续执行。
async fn install_and_restart(
    app: AppHandle,
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    log::info!("开始安装应用更新: {}", update.version);

    #[cfg(target_os = "windows")]
    {
        // Windows updater 会在 install() 内启动安装器并直接退出当前进程
        // （插件内部 std::process::exit(0)，绕过 TrayIcon::drop、不发
        // NIM_DELETE，会残留死图标——与托盘"退出"路径相同的问题）。
        // 因此清理只能放在 install 前执行，且必须显式移除托盘图标。
        crate::save_window_state_before_exit(&app);
        crate::cleanup_before_exit(&app).await;
        crate::remove_tray_icon_before_exit(&app);
        crate::destroy_single_instance_lock(&app);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        update.install(bytes).map_err(|e| {
            format!(
                "Windows 更新安装失败: {e}。已执行退出前清理，代理或 Live 接管可能已暂停；请重启应用或重新开启代理后再试。"
            )
        })?;
        Ok(true)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // macOS/Linux install() 会返回；先安装，避免安装失败时误停代理/撤回接管。
        update
            .install(bytes)
            .map_err(|e| format!("安装更新失败: {e}"))?;

        crate::save_window_state_before_exit(&app);
        crate::cleanup_before_exit(&app).await;

        log::info!("应用更新安装完成，正在重启应用");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        crate::restart_process(&app);
    }
}

/// 自动更新的状态：能不能自动安装、是否在下载、已下好待装的版本。
#[tauri::command]
pub async fn get_auto_update_status(
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<crate::app_update::AutoUpdateStatus, String> {
    Ok(crate::app_update::status(crate::tray::routing_in_use(
        state.inner(),
    )))
}

/// 立刻在后台检查并下载最新版本（打开自动更新开关时调用）。不支持自动安装的机器直接返回空。
#[tauri::command]
pub async fn download_app_update(
    app: AppHandle,
) -> Result<Option<crate::app_update::ReadyUpdate>, String> {
    if !crate::app_update::auto_install_supported() {
        return Ok(None);
    }
    crate::app_update::download_latest(&app).await
}

/// 检查是否有可用的应用更新，返回可用的新版本号（无更新时返回 None）。
///
/// 数据库版本过新的恢复界面用它判断：升级应用能否解决问题。若返回 None，说明
/// 已是最新版本，但数据库仍不兼容（通常由第三方客户端或更高版本创建），应提示用户
/// 升级无法解决，而不是让其反复尝试。
#[tauri::command]
pub async fn check_app_update_available(app: AppHandle) -> Result<Option<String>, String> {
    let updater = app
        .updater_builder()
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    Ok(update.map(|u| u.version))
}

/// 获取 app_config_dir 覆盖配置 (从 Store)
#[tauri::command]
pub async fn get_app_config_dir_override(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::app_store::refresh_app_config_dir_override(&app)
        .map(|p| p.to_string_lossy().to_string()))
}

/// 设置 app_config_dir 覆盖配置 (到 Store)
#[tauri::command]
pub async fn set_app_config_dir_override(
    app: AppHandle,
    path: Option<String>,
) -> Result<bool, String> {
    crate::app_store::set_app_config_dir_to_store(&app, path.as_deref())?;
    Ok(true)
}

#[tauri::command]
pub async fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    if enabled {
        crate::auto_launch::enable_auto_launch().map_err(|e| format!("启用开机自启失败: {e}"))?;
    } else {
        crate::auto_launch::disable_auto_launch().map_err(|e| format!("禁用开机自启失败: {e}"))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::merge_settings_for_save;
    use crate::settings::{
        AppSettings, CodexOfficialHistoryUnifyMigration, CodexProviderTemplateMigration,
        CodexThirdPartyHistoryProviderBucketMigration, LocalMigrations, S3SyncSettings,
        WebDavSyncSettings,
    };

    #[test]
    fn save_settings_should_keep_the_existing_lightweight_mode() {
        let existing = AppSettings {
            lightweight_mode: true,
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(AppSettings::default(), &existing);

        assert!(merged.lightweight_mode);
    }

    #[test]
    fn save_settings_should_keep_the_existing_proxy_client_host() {
        let existing = AppSettings {
            proxy_client_host: Some("172.25.144.1".to_string()),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(AppSettings::default(), &existing);

        assert_eq!(merged.proxy_client_host.as_deref(), Some("172.25.144.1"));
    }

    #[test]
    fn save_settings_should_preserve_existing_webdav_when_payload_omits_it() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.webdav_sync.is_some());
        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.example.com")
        );
    }

    #[test]
    fn save_settings_should_keep_incoming_webdav_when_present() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.old.example.com".to_string(),
                username: "old".to_string(),
                password: "old-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.new.example.com".to_string(),
                username: "new".to_string(),
                password: "new-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.new.example.com")
        );
    }

    /// Regression test: frontend always receives empty password from
    /// get_settings_for_frontend(). If a component accidentally spreads
    /// the full settings object into save_settings, the empty password
    /// must NOT overwrite the existing one.
    #[test]
    fn save_settings_should_preserve_password_when_incoming_has_empty_password() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        // Simulate frontend sending settings with cleared password
        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("secret"),
            "empty password from frontend must not overwrite existing password"
        );
    }

    /// When both incoming and existing have no password, merge should
    /// work without panicking and keep the empty state.
    #[test]
    fn save_settings_should_handle_both_empty_passwords() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("")
        );
    }

    #[test]
    fn save_settings_should_preserve_existing_s3_when_payload_omits_it() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.s3_sync.is_some());
        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_s3_secret_when_incoming_has_empty_secret() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_local_migrations_when_payload_omits_it() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: Some(
                    CodexThirdPartyHistoryProviderBucketMigration {
                        completed_at: "2026-05-20T00:00:00Z".to_string(),
                        target_provider_id: "custom".to_string(),
                        source_provider_ids: vec!["rightcode".to_string()],
                        migrated_jsonl_files: 2,
                        migrated_state_rows: 3,
                        scanned_history_files: true,
                    },
                ),
                codex_provider_template_v1: Some(CodexProviderTemplateMigration {
                    completed_at: "2026-05-20T00:01:00Z".to_string(),
                    migrated_provider_ids: vec!["legacy".to_string()],
                }),
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    source_provider_ids: Vec::new(),
                    migrated_jsonl_files: 5,
                    migrated_state_rows: 7,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        let migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| {
                migrations
                    .codex_third_party_history_provider_bucket_v1
                    .as_ref()
            })
            .expect("local migration marker should be preserved");
        assert_eq!(migration.target_provider_id, "custom");
        assert_eq!(migration.migrated_jsonl_files, 2);
        assert_eq!(migration.migrated_state_rows, 3);

        let template_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_provider_template_v1.as_ref())
            .expect("template migration marker should be preserved");
        assert_eq!(
            template_migration.migrated_provider_ids,
            vec!["legacy".to_string()]
        );

        let unify_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .expect("official unify migration marker should be preserved");
        assert_eq!(unify_migration.migrated_jsonl_files, 5);
        assert_eq!(unify_migration.migrated_state_rows, 7);
    }

    /// incoming 带有 local_migrations（哪怕是空的）也不能覆盖后端维护的标记。
    #[test]
    fn save_settings_should_keep_backend_migration_markers_over_incoming() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: None,
                codex_provider_template_v1: None,
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    source_provider_ids: Vec::new(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations::default()),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .is_some());
    }

    /// 后端清掉 marker 后（如关闭统一会话开关）、前端缓存刷新前的全量保存
    /// 会携带旧 marker；merge 必须忽略它，否则被"复活"的标记会让重新开启
    /// 时误判已迁移而漏迁。
    #[test]
    fn save_settings_should_ignore_stale_incoming_migration_markers() {
        let existing = AppSettings::default();

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    source_provider_ids: Vec::new(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.local_migrations.is_none());
    }
}

#[tauri::command]
pub async fn get_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::RectifierConfig, AppError> {
    state.db.get_rectifier_config()
}

#[tauri::command]
pub async fn set_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::RectifierConfig,
) -> Result<bool, String> {
    state.db.set_rectifier_config(&config)?;
    Ok(true)
}

#[tauri::command]
pub async fn get_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::OptimizerConfig, AppError> {
    state.db.get_optimizer_config()
}

#[tauri::command]
pub async fn set_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::OptimizerConfig,
) -> Result<bool, String> {
    state.db.set_optimizer_config(&config)?;
    Ok(true)
}

#[tauri::command]
pub async fn get_privacy_filter_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::PrivacyFilterConfig, AppError> {
    state.db.get_privacy_filter_config()
}

#[tauri::command]
pub async fn set_privacy_filter_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::PrivacyFilterConfig,
) -> Result<bool, String> {
    state.db.set_privacy_filter_config(&config)?;
    if config.enabled {
        // 规则编译有一次性耗时，开关打开时在后台预热，不压到第一个请求上
        std::thread::spawn(crate::proxy::privacy_filter::warm_up);
    }
    Ok(true)
}

#[tauri::command]
pub async fn get_log_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::LogConfig, AppError> {
    state.db.get_log_config()
}

#[tauri::command]
pub async fn set_log_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::LogConfig,
) -> Result<bool, String> {
    state.db.set_log_config(&config)?;
    log::set_max_level(config.to_level_filter());
    log::info!(
        "日志配置已更新: enabled={}, level={}",
        config.enabled,
        config.level
    );
    Ok(true)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAppRestartResult {
    pub was_running: bool,
    pub launched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

/// 重启承载 Codex 的 ChatGPT App；旧版独立 Codex App 仍作为兼容回退。
#[tauri::command]
pub async fn restart_chatgpt_app(
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<CodexAppRestartResult, String> {
    restart_chatgpt_app_impl(state.inner()).await
}

/// ChatGPT/Codex 会在退出时回写部分本地状态。必须等旧进程完全退出后，再以
/// 当前供应商为 SSOT 重投影认证和路由，最后启动新进程，避免旧状态覆盖新 Key。
fn sync_current_codex_provider_before_external_launch(
    state: &crate::store::AppState,
) -> Result<(), String> {
    crate::services::provider::ProviderService::sync_current_codex_provider_for_external_restart(
        state,
    )
    .map_err(|err| format!("启动前写入当前 Codex 认证配置失败: {err}"))
}

#[cfg(target_os = "macos")]
async fn restart_chatgpt_app_impl(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    let state = state.clone();
    tauri::async_runtime::spawn_blocking(move || restart_chatgpt_or_codex_app_macos(&state))
        .await
        .map_err(|e| format!("重启 ChatGPT/Codex App 任务失败: {e}"))?
}

#[cfg(target_os = "macos")]
fn restart_chatgpt_or_codex_app_macos(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    if resolve_chatgpt_app_path().is_some() {
        restart_chatgpt_app_macos(state)
    } else {
        restart_codex_app_macos(state)
    }
}

#[cfg(target_os = "macos")]
fn restart_chatgpt_app_macos(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    let app_path = resolve_chatgpt_app_path();
    let had_processes = chatgpt_app_has_processes(app_path.as_deref())?;
    let was_running = chatgpt_app_is_running(app_path.as_deref()).unwrap_or(had_processes);

    if was_running || had_processes {
        if let Err(err) = quit_chatgpt_app(app_path.as_deref()) {
            log::warn!("ChatGPT graceful quit failed, will terminate processes if needed: {err}");
        }

        if !wait_for_chatgpt_process_state(
            app_path.as_deref(),
            false,
            std::time::Duration::from_secs(5),
        )? {
            signal_codex_app_processes(app_path.as_deref(), "-TERM")?;
            if !wait_for_chatgpt_process_state(
                app_path.as_deref(),
                false,
                std::time::Duration::from_secs(10),
            )? {
                signal_codex_app_processes(app_path.as_deref(), "-KILL")?;
                if !wait_for_chatgpt_process_state(
                    app_path.as_deref(),
                    false,
                    std::time::Duration::from_secs(5),
                )? {
                    return Err("等待 ChatGPT App 退出超时".to_string());
                }
            }
        }
    }

    sync_current_codex_provider_before_external_launch(state)?;
    launch_chatgpt_app(app_path.as_deref())?;
    if !wait_for_chatgpt_process_state(
        app_path.as_deref(),
        true,
        std::time::Duration::from_secs(15),
    )? && !chatgpt_app_is_running(app_path.as_deref())?
    {
        return Err("等待 ChatGPT App 启动超时".to_string());
    }

    Ok(CodexAppRestartResult {
        was_running,
        launched: true,
        app_path: app_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        app_id: Some(chatgpt_app_bundle_id(app_path.as_deref())),
    })
}

#[cfg(target_os = "macos")]
fn resolve_chatgpt_app_path() -> Option<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from("/Applications/ChatGPT.app")];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications").join("ChatGPT.app"));
    }

    candidates.into_iter().find(|path| path.exists())
}

#[cfg(target_os = "macos")]
fn chatgpt_app_bundle_id(app_path: Option<&std::path::Path>) -> String {
    let Some(app_path) = app_path else {
        return "com.openai.chat".to_string();
    };

    let plist_path = app_path.join("Contents").join("Info.plist");
    let output = std::process::Command::new("defaults")
        .arg("read")
        .arg(plist_path)
        .arg("CFBundleIdentifier")
        .output();
    let Ok(output) = output else {
        return "com.openai.chat".to_string();
    };
    if !output.status.success() {
        return "com.openai.chat".to_string();
    }

    let bundle_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if bundle_id.is_empty()
        || !bundle_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
    {
        "com.openai.chat".to_string()
    } else {
        bundle_id
    }
}

#[cfg(target_os = "macos")]
fn chatgpt_app_is_running(app_path: Option<&std::path::Path>) -> Result<bool, String> {
    let bundle_id = chatgpt_app_bundle_id(app_path);
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(r#"application id "{bundle_id}" is running"#))
        .output()
        .map_err(|e| format!("检查 ChatGPT App 运行状态失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "检查 ChatGPT App 运行状态失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

#[cfg(target_os = "macos")]
fn quit_chatgpt_app(app_path: Option<&std::path::Path>) -> Result<(), String> {
    let bundle_id = chatgpt_app_bundle_id(app_path);
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(r#"tell application id "{bundle_id}" to quit"#))
        .output()
        .map_err(|e| format!("退出 ChatGPT App 失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "退出 ChatGPT App 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn chatgpt_app_has_processes(app_path: Option<&std::path::Path>) -> Result<bool, String> {
    match app_path {
        Some(path) => Ok(!codex_app_pids(path)?.is_empty()),
        None => chatgpt_app_is_running(None),
    }
}

#[cfg(target_os = "macos")]
fn wait_for_chatgpt_process_state(
    app_path: Option<&std::path::Path>,
    expected: bool,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if chatgpt_app_has_processes(app_path)? == expected {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    Ok(false)
}

#[cfg(target_os = "macos")]
fn launch_chatgpt_app(app_path: Option<&std::path::Path>) -> Result<(), String> {
    let mut command = std::process::Command::new("open");
    if let Some(path) = app_path {
        command.arg(path);
    } else {
        command.arg("-b").arg("com.openai.chat");
    }

    let output = command
        .output()
        .map_err(|e| format!("启动 ChatGPT App 失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "启动 ChatGPT App 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn restart_codex_app_macos(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    let app_path = resolve_codex_app_path();
    let had_processes = codex_app_has_processes(app_path.as_deref())?;
    let was_running = codex_app_is_running().unwrap_or(had_processes);

    if was_running || had_processes {
        if let Err(err) = quit_codex_app() {
            log::warn!("Codex App graceful quit failed, will terminate processes if needed: {err}");
        }

        if !wait_for_codex_process_state(
            app_path.as_deref(),
            false,
            std::time::Duration::from_secs(5),
        )? {
            signal_codex_app_processes(app_path.as_deref(), "-TERM")?;
            if !wait_for_codex_process_state(
                app_path.as_deref(),
                false,
                std::time::Duration::from_secs(10),
            )? {
                signal_codex_app_processes(app_path.as_deref(), "-KILL")?;
                if !wait_for_codex_process_state(
                    app_path.as_deref(),
                    false,
                    std::time::Duration::from_secs(5),
                )? {
                    return Err("等待 Codex App 退出超时".to_string());
                }
            }
        }
    }

    sync_current_codex_provider_before_external_launch(state)?;
    launch_codex_app(app_path.as_deref())?;
    if !wait_for_codex_process_state(
        app_path.as_deref(),
        true,
        std::time::Duration::from_secs(15),
    )? && !codex_app_is_running()?
    {
        return Err("等待 Codex App 启动超时".to_string());
    }

    Ok(CodexAppRestartResult {
        was_running,
        launched: true,
        app_path: app_path.map(|path| path.to_string_lossy().to_string()),
        app_id: None,
    })
}

#[cfg(target_os = "macos")]
fn resolve_codex_app_path() -> Option<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from("/Applications/Codex.app")];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications").join("Codex.app"));
    }

    candidates.into_iter().find(|path| path.exists())
}

#[cfg(target_os = "macos")]
fn codex_app_is_running() -> Result<bool, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(r#"application id "com.openai.codex" is running"#)
        .output()
        .map_err(|e| format!("检查 Codex App 运行状态失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "检查 Codex App 运行状态失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

#[cfg(target_os = "macos")]
fn quit_codex_app() -> Result<(), String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application id "com.openai.codex" to quit"#)
        .output()
        .map_err(|e| format!("退出 Codex App 失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "退出 Codex App 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn codex_app_has_processes(app_path: Option<&std::path::Path>) -> Result<bool, String> {
    match app_path {
        Some(path) => Ok(!codex_app_pids(path)?.is_empty()),
        None => codex_app_is_running(),
    }
}

#[cfg(target_os = "macos")]
fn wait_for_codex_process_state(
    app_path: Option<&std::path::Path>,
    expected: bool,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if codex_app_has_processes(app_path)? == expected {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    Ok(false)
}

#[cfg(target_os = "macos")]
fn codex_app_pids(app_path: &std::path::Path) -> Result<Vec<u32>, String> {
    let marker = app_path.to_string_lossy();
    let output = std::process::Command::new("ps")
        .args(["ax", "-o", "pid=,args="])
        .output()
        .map_err(|e| format!("读取 Codex App 进程失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "读取 Codex App 进程失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in stdout.lines() {
        if !line.contains(marker.as_ref()) {
            continue;
        }

        let trimmed = line.trim_start();
        let Some((pid_text, _args)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        if let Ok(pid) = pid_text.parse::<u32>() {
            pids.push(pid);
        }
    }

    Ok(pids)
}

#[cfg(target_os = "macos")]
fn signal_codex_app_processes(
    app_path: Option<&std::path::Path>,
    signal: &str,
) -> Result<(), String> {
    let Some(path) = app_path else {
        return Ok(());
    };
    let pids = codex_app_pids(path)?;
    if pids.is_empty() {
        return Ok(());
    }

    let mut command = std::process::Command::new("kill");
    command.arg(signal);
    for pid in pids {
        command.arg(pid.to_string());
    }
    let output = command
        .output()
        .map_err(|e| format!("终止 Codex App 进程失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "终止 Codex App 进程失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_codex_app(app_path: Option<&std::path::Path>) -> Result<(), String> {
    let mut command = std::process::Command::new("open");
    if let Some(path) = app_path {
        command.arg(path);
    } else {
        command.arg("-b").arg("com.openai.codex");
    }

    let output = command
        .output()
        .map_err(|e| format!("启动 Codex App 失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "启动 Codex App 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
async fn restart_chatgpt_app_impl(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    let state = state.clone();
    tauri::async_runtime::spawn_blocking(move || restart_chatgpt_or_codex_app_windows(&state))
        .await
        .map_err(|e| format!("重启 ChatGPT/Codex App 任务失败: {e}"))?
}

#[cfg(target_os = "windows")]
fn restart_chatgpt_or_codex_app_windows(
    state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    let launch_target = match resolve_windows_chatgpt_launch_target() {
        Ok(target) => target,
        Err(chatgpt_err) => {
            log::info!("Windows ChatGPT App unavailable, falling back to Codex App: {chatgpt_err}");
            resolve_windows_codex_launch_target()?
        }
    };

    restart_chatgpt_or_codex_app_windows_with_target(state, &launch_target)
}

#[cfg(target_os = "windows")]
fn restart_chatgpt_or_codex_app_windows_with_target(
    state: &crate::store::AppState,
    launch_target: &WindowsCodexLaunchTarget,
) -> Result<CodexAppRestartResult, String> {
    // New ChatGPT builds can retain the OpenAI.Codex package identity while
    // renaming the desktop executable to ChatGPT.exe. Process detection must
    // therefore be independent from which package/AppID resolved the launch.
    let running_processes = windows_codex_desktop_process_snapshot()?;
    let was_running = running_processes.has_processes();

    if was_running {
        quit_windows_codex_desktop_processes(&running_processes)?;
    }

    sync_current_codex_provider_before_external_launch(state)?;
    let process_baseline = windows_codex_desktop_process_snapshot()?;
    start_windows_app(launch_target, "ChatGPT/Codex App")?;
    if !wait_for_new_windows_codex_desktop_process(
        &process_baseline,
        std::time::Duration::from_secs(12),
    )? {
        return Err(format!(
            "已请求启动 Windows ChatGPT/Codex App（{}），但未检测到新的 ChatGPT.exe 或 Codex.exe 进程",
            launch_target.label()
        ));
    }

    Ok(CodexAppRestartResult {
        was_running,
        launched: true,
        app_path: launch_target.app_path(),
        app_id: launch_target.app_id(),
    })
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
enum WindowsCodexLaunchTarget {
    AppId(String),
    ExecutablePath(std::path::PathBuf),
}

#[cfg(target_os = "windows")]
impl WindowsCodexLaunchTarget {
    fn app_id(&self) -> Option<String> {
        match self {
            Self::AppId(app_id) => Some(app_id.clone()),
            Self::ExecutablePath(_) => None,
        }
    }

    fn app_path(&self) -> Option<String> {
        match self {
            Self::AppId(_) => None,
            Self::ExecutablePath(path) => Some(path.to_string_lossy().to_string()),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::AppId(app_id) => app_id.clone(),
            Self::ExecutablePath(path) => path.to_string_lossy().to_string(),
        }
    }
}

#[cfg(target_os = "windows")]
fn resolve_windows_codex_launch_target() -> Result<WindowsCodexLaunchTarget, String> {
    let mut failures = Vec::new();

    match windows_codex_app_id() {
        Ok(app_id) => return Ok(WindowsCodexLaunchTarget::AppId(app_id)),
        Err(err) => failures.push(err),
    }

    if let Some(path) = windows_codex_app_alias_path() {
        return Ok(WindowsCodexLaunchTarget::ExecutablePath(path));
    }

    Err(format!(
        "未能定位 Windows Codex App。请先安装 Codex App，或通过安装功能自动安装。详细：{}",
        failures.join("；")
    ))
}

#[cfg(target_os = "windows")]
fn resolve_windows_chatgpt_launch_target() -> Result<WindowsCodexLaunchTarget, String> {
    let mut failures = Vec::new();

    match windows_chatgpt_app_id() {
        Ok(app_id) => return Ok(WindowsCodexLaunchTarget::AppId(app_id)),
        Err(err) => failures.push(err),
    }

    if let Some(path) = windows_chatgpt_app_alias_path() {
        return Ok(WindowsCodexLaunchTarget::ExecutablePath(path));
    }

    Err(format!(
        "未能定位 Windows ChatGPT App。详细：{}",
        failures.join("；")
    ))
}

#[cfg(target_os = "windows")]
fn windows_chatgpt_app_id() -> Result<String, String> {
    let mut failures = Vec::new();

    match windows_chatgpt_app_id_from_powershell() {
        Ok(app_id) => return Ok(app_id),
        Err(err) => failures.push(format!("PowerShell 查询失败：{err}")),
    }

    match windows_app_id_from_manifest_dirs("openai.chatgpt_", "OpenAI.ChatGPT") {
        Ok(app_id) => return Ok(app_id),
        Err(err) => failures.push(format!("WindowsApps manifest 查询失败：{err}")),
    }

    Err(failures.join("；"))
}

#[cfg(target_os = "windows")]
fn windows_chatgpt_app_id_from_powershell() -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = "Stop"

function Test-IsSwitcher($entry) {
  $name = [string]$entry.Name
  $appId = [string]$entry.AppID
  return (
    $name -match "(?i)CCodexSwitch|CodexSwitch|Codex Switch|Profile Switcher|Codex Account Switcher|Account Switcher|切号器" -or
    $appId -match "(?i)ccc-switch|cc-switch|codex-switch|profile-switcher|codex-account-switcher|com\.local"
  )
}

$startApps = @(Get-StartApps | Where-Object { -not (Test-IsSwitcher $_) })
$packages = @(Get-AppxPackage -Name "OpenAI.ChatGPT" -ErrorAction SilentlyContinue)
if ($packages.Count -eq 0) {
  $packages = @(Get-AppxPackage -ErrorAction SilentlyContinue |
    Where-Object {
      $_.Name -eq "OpenAI.ChatGPT" -or
      $_.PackageFamilyName -like "OpenAI.ChatGPT_*"
    })
}

foreach ($package in $packages) {
  $app = $startApps |
    Where-Object { $_.AppID -like "$($package.PackageFamilyName)!*" } |
    Select-Object -First 1
  if ($app) {
    $app.AppID
    exit 0
  }
}

$app = $startApps |
  Where-Object {
    $_.Name -eq "ChatGPT" -and (
      $_.AppID -match "(?i)^OpenAI\.ChatGPT_" -or
      $_.AppID -match "(?i)9PLM9XGG6VKS"
    )
  } |
  Select-Object -First 1

if (-not $app) {
  $app = $startApps |
    Where-Object {
      $_.Name -eq "ChatGPT" -and
      $_.AppID -match "!" -and
      $_.AppID -notmatch "(?i)switcher|account-switcher|codex-account-switcher|ccc-switch|cc-switch|codex-switch|com\.local"
    } |
    Select-Object -First 1
}

if (-not $app) { exit 1 }
$app.AppID
"#;
    let output = windows_powershell_stdout(script)?;
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "未能从 Windows 开始菜单读取 ChatGPT AppID".to_string())
}

#[cfg(target_os = "windows")]
fn windows_codex_app_id() -> Result<String, String> {
    let mut failures = Vec::new();

    match windows_codex_app_id_from_powershell() {
        Ok(app_id) => return Ok(app_id),
        Err(err) => failures.push(format!("PowerShell 查询失败：{err}")),
    }

    match windows_codex_app_id_from_manifest_dirs() {
        Ok(app_id) => return Ok(app_id),
        Err(err) => failures.push(format!("WindowsApps manifest 查询失败：{err}")),
    }

    Err(failures.join("；"))
}

#[cfg(target_os = "windows")]
fn windows_codex_app_id_from_powershell() -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = "Stop"

function Test-IsSwitcher($entry) {
  $name = [string]$entry.Name
  $appId = [string]$entry.AppID
  return (
    $name -match "(?i)CCodexSwitch|CodexSwitch|Codex Switch|Profile Switcher|Codex Account Switcher|Account Switcher|切号器" -or
    $appId -match "(?i)ccc-switch|cc-switch|codex-switch|profile-switcher|codex-account-switcher|com\.local"
  )
}

$startApps = @(Get-StartApps | Where-Object { -not (Test-IsSwitcher $_) })
$packages = @(Get-AppxPackage -Name "OpenAI.Codex" -ErrorAction SilentlyContinue)
if ($packages.Count -eq 0) {
  $packages = @(Get-AppxPackage -ErrorAction SilentlyContinue |
    Where-Object {
      $_.Name -eq "OpenAI.Codex" -or
      $_.PackageFamilyName -like "OpenAI.Codex_*"
    })
}

foreach ($package in $packages) {
  $app = $startApps |
    Where-Object { $_.AppID -like "$($package.PackageFamilyName)!*" } |
    Select-Object -First 1
  if ($app) {
    $app.AppID
    exit 0
  }
}

$app = $startApps |
  Where-Object {
    $_.Name -eq "Codex" -and (
      $_.AppID -match "(?i)^OpenAI\.Codex_" -or
      $_.AppID -match "(?i)9PLM9XGG6VKS"
    )
  } |
  Select-Object -First 1

if (-not $app) {
  $app = $startApps |
    Where-Object {
      $_.Name -eq "Codex" -and
      $_.AppID -match "!" -and
      $_.AppID -notmatch "(?i)switcher|account-switcher|codex-account-switcher|ccc-switch|cc-switch|codex-switch|com\.local"
    } |
    Select-Object -First 1
}

if (-not $app) { exit 1 }
$app.AppID
"#;
    let output = windows_powershell_stdout(script)?;
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "未能从 Windows 开始菜单读取 Codex AppID".to_string())
}

#[cfg(target_os = "windows")]
fn windows_codex_app_id_from_manifest_dirs() -> Result<String, String> {
    windows_app_id_from_manifest_dirs("openai.codex_", "OpenAI.Codex")
}

#[cfg(target_os = "windows")]
fn windows_app_id_from_manifest_dirs(
    package_prefix: &str,
    package_label: &str,
) -> Result<String, String> {
    let mut roots = Vec::new();
    if let Some(program_files) =
        std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles"))
    {
        roots.push(std::path::PathBuf::from(program_files).join("WindowsApps"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        roots.push(
            std::path::PathBuf::from(local_app_data)
                .join("Microsoft")
                .join("WindowsApps"),
        );
    }

    let mut failures = Vec::new();
    for apps_root in roots {
        let entries = match std::fs::read_dir(&apps_root) {
            Ok(entries) => entries,
            Err(err) => {
                failures.push(format!("读取 {} 失败：{err}", apps_root.display()));
                continue;
            }
        };

        for entry in entries.flatten() {
            let package_dir = entry.path();
            if !package_dir.is_dir() {
                continue;
            }

            let Some(dir_name) = package_dir.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !dir_name
                .to_ascii_lowercase()
                .starts_with(&package_prefix.to_ascii_lowercase())
            {
                continue;
            }

            let manifest_path = package_dir.join("AppxManifest.xml");
            let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Some(application_id) = extract_windows_appx_application_id(&manifest) else {
                continue;
            };
            return Ok(format!("{dir_name}!{application_id}"));
        }
    }

    Err(if failures.is_empty() {
        format!("未在 WindowsApps manifest 中找到 {package_label} AppID")
    } else {
        format!(
            "未在 WindowsApps manifest 中找到 {package_label} AppID；{}",
            failures.join("；")
        )
    })
}

#[cfg(target_os = "windows")]
fn extract_windows_appx_application_id(manifest: &str) -> Option<String> {
    let application_pos = manifest.find("<Application")?;
    let id_pos = manifest[application_pos..].find("Id=")? + application_pos;
    let after_id = &manifest[id_pos + "Id=".len()..];
    let quote = after_id.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after_id[quote.len_utf8()..];
    let end = rest.find(quote)?;
    let id = rest[..end].trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_codex_app_alias_path() -> Option<std::path::PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let candidate = std::path::PathBuf::from(local_app_data)
        .join("Microsoft")
        .join("WindowsApps")
        .join("Codex.exe");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_chatgpt_app_alias_path() -> Option<std::path::PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let candidate = std::path::PathBuf::from(local_app_data)
        .join("Microsoft")
        .join("WindowsApps")
        .join("ChatGPT.exe");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_codex_app_process_ids() -> Result<Vec<String>, String> {
    let mut failures = Vec::new();

    match windows_codex_app_process_ids_from_powershell() {
        Ok(process_ids) => return Ok(process_ids),
        Err(err) => failures.push(format!("PowerShell 查询失败：{err}")),
    }

    match windows_codex_app_process_ids_from_tasklist() {
        Ok(process_ids) => return Ok(process_ids),
        Err(err) => failures.push(format!("tasklist 查询失败：{err}")),
    }

    Err(failures.join("；"))
}

#[cfg(target_os = "windows")]
fn windows_codex_app_process_ids_from_powershell() -> Result<Vec<String>, String> {
    let script = r#"
$ErrorActionPreference = "Stop"
$processes = Get-CimInstance Win32_Process -Filter "Name = 'Codex.exe'" |
  Where-Object {
    ($_.ExecutablePath -and ($_.ExecutablePath -match "(?i)\\WindowsApps\\" -or $_.ExecutablePath -match "(?i)\\Programs\\Codex\\" -or $_.ExecutablePath -match "(?i)Codex App")) -or
    ($_.CommandLine -and ($_.CommandLine -match "(?i)\\WindowsApps\\" -or $_.CommandLine -match "(?i)Codex App" -or $_.CommandLine -match "(?i)ms-appx"))
  } |
  Select-Object -ExpandProperty ProcessId
$processes
"#;
    let output = windows_powershell_stdout(script)?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(target_os = "windows")]
fn windows_codex_app_process_ids_from_tasklist() -> Result<Vec<String>, String> {
    windows_process_ids_by_image("Codex.exe")
}

#[cfg(target_os = "windows")]
fn windows_process_ids_by_image(image_name: &str) -> Result<Vec<String>, String> {
    let filter = format!("IMAGENAME eq {image_name}");
    let output = windows_command_stdout("tasklist", &["/FI", &filter, "/FO", "CSV", "/NH"])?;
    Ok(output
        .lines()
        .filter_map(|line| parse_tasklist_csv_pid_for_image(line, image_name))
        .collect())
}

#[cfg(target_os = "windows")]
fn parse_tasklist_csv_pid(line: &str) -> Option<String> {
    parse_tasklist_csv_pid_for_image(line, "Codex.exe")
}

#[cfg(target_os = "windows")]
fn parse_tasklist_csv_pid_for_image(line: &str, image_name: &str) -> Option<String> {
    let columns = parse_windows_csv_line(line);
    if columns.len() < 2 {
        return None;
    }
    if !columns[0].eq_ignore_ascii_case(image_name) {
        return None;
    }
    let pid = columns[1].trim();
    if pid.chars().all(|ch| ch.is_ascii_digit()) {
        Some(pid.to_string())
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn parse_windows_csv_line(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                columns.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    columns.push(current.trim().to_string());
    columns
}

#[cfg(target_os = "windows")]
fn codex_windows_process_exists() -> Result<bool, String> {
    windows_codex_app_process_ids().map(|ids| !ids.is_empty())
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct WindowsCodexDesktopProcessSnapshot {
    chatgpt_ids: Vec<String>,
    codex_ids: Vec<String>,
}

#[cfg(target_os = "windows")]
impl WindowsCodexDesktopProcessSnapshot {
    fn has_processes(&self) -> bool {
        !self.chatgpt_ids.is_empty() || !self.codex_ids.is_empty()
    }

    fn has_new_process_since(&self, baseline: &Self) -> bool {
        self.chatgpt_ids
            .iter()
            .any(|pid| !baseline.chatgpt_ids.contains(pid))
            || self
                .codex_ids
                .iter()
                .any(|pid| !baseline.codex_ids.contains(pid))
    }
}

#[cfg(target_os = "windows")]
fn windows_codex_desktop_process_snapshot() -> Result<WindowsCodexDesktopProcessSnapshot, String> {
    Ok(WindowsCodexDesktopProcessSnapshot {
        chatgpt_ids: windows_process_ids_by_image("ChatGPT.exe")?,
        codex_ids: windows_codex_app_process_ids()?,
    })
}

#[cfg(target_os = "windows")]
fn quit_windows_codex_desktop_processes(
    running: &WindowsCodexDesktopProcessSnapshot,
) -> Result<(), String> {
    if !running.chatgpt_ids.is_empty() {
        if let Err(err) =
            windows_command_status_detail("taskkill", &["/F", "/T", "/IM", "ChatGPT.exe"])
        {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if windows_process_exists_by_image("ChatGPT.exe").unwrap_or(true) {
                return Err(format!("退出 Windows ChatGPT App 失败：{}", err.detail()));
            }
            log::warn!(
                "taskkill 结束 ChatGPT.exe 返回异常，但进程已经退出：{}",
                err.detail()
            );
        }

        if !wait_for_windows_process_state_by_image(
            "ChatGPT.exe",
            false,
            std::time::Duration::from_secs(10),
        )? {
            return Err("等待 Windows ChatGPT App 退出超时，请手动关闭应用后重试。".to_string());
        }
    }

    // ChatGPT and the legacy Codex desktop app share ~/.codex. If both are
    // present, the survivor can write its cached auth back after we switch.
    if !running.codex_ids.is_empty() && codex_windows_process_exists()? {
        quit_codex_app_windows()?;
        if !wait_for_windows_codex_process_state(false, std::time::Duration::from_secs(10))? {
            return Err("等待 Windows Codex App 退出超时，请手动关闭应用后重试。".to_string());
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn wait_for_new_windows_codex_desktop_process(
    baseline: &WindowsCodexDesktopProcessSnapshot,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if windows_codex_desktop_process_snapshot()?.has_new_process_since(baseline) {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    Ok(false)
}

#[cfg(target_os = "windows")]
fn windows_process_exists_by_image(image_name: &str) -> Result<bool, String> {
    windows_process_ids_by_image(image_name).map(|ids| !ids.is_empty())
}

#[cfg(target_os = "windows")]
fn wait_for_windows_process_state_by_image(
    image_name: &str,
    expected: bool,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if windows_process_exists_by_image(image_name)? == expected {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    Ok(false)
}

#[cfg(target_os = "windows")]
fn wait_for_windows_codex_process_state(
    expected: bool,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if codex_windows_process_exists()? == expected {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    Ok(false)
}

#[cfg(target_os = "windows")]
fn quit_codex_app_windows() -> Result<(), String> {
    let mut failures = Vec::new();
    let mut any_success = false;
    let mut access_denied = false;

    match codex_windows_process_exists() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(err) => failures.push(format!("检测 Codex.exe 进程失败：{err}")),
    }

    match windows_codex_app_process_ids() {
        Ok(process_ids) if process_ids.is_empty() => return Ok(()),
        Ok(process_ids) => {
            for process_id in process_ids {
                let close_script = format!(
                    r#"
$ErrorActionPreference = "SilentlyContinue"
$process = Get-Process -Id {} -ErrorAction SilentlyContinue
if ($process) {{ $process.CloseMainWindow() | Out-Null }}
Start-Sleep -Milliseconds 900
"#,
                    process_id
                );
                if let Err(err) = windows_powershell_status(&close_script) {
                    failures.push(format!("温和关闭 PID {process_id} 失败：{err}"));
                }
            }
        }
        Err(err) => failures.push(format!("读取 Codex App 进程失败：{err}")),
    }

    match codex_windows_process_exists() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(err) => failures.push(format!("温和关闭后检测进程失败：{err}")),
    }

    match windows_codex_app_process_ids() {
        Ok(process_ids) if process_ids.is_empty() => return Ok(()),
        Ok(process_ids) => {
            for process_id in process_ids {
                match windows_command_status_detail("taskkill", &["/F", "/T", "/PID", &process_id])
                {
                    Ok(()) => any_success = true,
                    Err(err) => {
                        any_success = any_success || err.has_successful_termination();
                        access_denied = access_denied || err.is_access_denied();
                        failures.push(format!("taskkill PID {process_id} 失败：{}", err.detail()));
                    }
                }
            }
        }
        Err(err) => failures.push(format!("读取 Codex App 进程失败：{err}")),
    }
    std::thread::sleep(std::time::Duration::from_millis(800));

    match codex_windows_process_exists() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(err) => failures.push(format!("taskkill 后检测进程失败：{err}")),
    }

    let script = r#"
$ErrorActionPreference = "Continue"
Get-CimInstance Win32_Process -Filter "Name = 'Codex.exe'" |
  Where-Object {
    ($_.ExecutablePath -and ($_.ExecutablePath -match "(?i)\\WindowsApps\\" -or $_.ExecutablePath -match "(?i)\\Programs\\Codex\\" -or $_.ExecutablePath -match "(?i)Codex App")) -or
    ($_.CommandLine -and ($_.CommandLine -match "(?i)\\WindowsApps\\" -or $_.CommandLine -match "(?i)Codex App" -or $_.CommandLine -match "(?i)ms-appx"))
  } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction Continue }
"#;
    match windows_powershell_status(script) {
        Ok(()) => any_success = true,
        Err(err) => {
            access_denied = access_denied || text_has_access_denied(&err);
            failures.push(format!("PowerShell 停止进程失败：{err}"));
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(1200));

    let final_process_state = codex_windows_process_exists();
    match final_process_state {
        Ok(false) => Ok(()),
        Ok(true) if access_denied => Err(format!(
            "Codex App 仍在运行，Windows 拒绝当前应用结束部分 Codex 进程。请以管理员身份重启 CCodexSwitch，或手动关闭 Codex App 后重试。{}",
            if any_success {
                "已成功关闭一部分 Codex 子进程，但仍有进程需要更高权限。".to_string()
            } else {
                String::new()
            }
        )),
        Ok(true) => Err(format!(
            "已尝试温和关闭、taskkill 和 PowerShell 停止 Codex App，但 Codex.exe 仍在运行。请手动关闭 Codex App 后重试。{}",
            if failures.is_empty() {
                String::new()
            } else {
                format!(" 详细：{}", failures.join("；"))
            }
        )),
        Err(err) if any_success => Ok(()),
        Err(err) => Err(format!(
            "无法确认 Codex App 是否已关闭：{err}。{}",
            if failures.is_empty() {
                "请手动关闭 Codex App 后重试。".to_string()
            } else {
                format!("详细：{}", failures.join("；"))
            }
        )),
    }
}

#[cfg(target_os = "windows")]
fn start_windows_app(target: &WindowsCodexLaunchTarget, app_name: &str) -> Result<(), String> {
    match target {
        WindowsCodexLaunchTarget::AppId(app_id) => hidden_command("explorer.exe")
            .arg(format!("shell:AppsFolder\\{app_id}"))
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("未能通过 Windows AppID 启动 {app_name}（{app_id}）：{err}")),
        WindowsCodexLaunchTarget::ExecutablePath(path) => {
            hidden_command(path).spawn().map(|_| ()).map_err(|err| {
                format!(
                    "未能通过 Windows App Execution Alias 启动 {app_name}（{}）：{err}",
                    path.display()
                )
            })
        }
    }
}

#[cfg(target_os = "windows")]
fn hidden_command<P>(program: P) -> std::process::Command
where
    P: AsRef<std::ffi::OsStr>,
{
    let mut command = std::process::Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct WindowsCommandError {
    program: String,
    status: String,
    stdout: String,
    stderr: String,
}

#[cfg(target_os = "windows")]
impl WindowsCommandError {
    fn detail(&self) -> String {
        let mut parts = Vec::new();
        if !self.stdout.trim().is_empty() {
            parts.push(format!(
                "stdout: {}",
                first_non_empty_lines(&self.stdout, 4)
            ));
        }
        if !self.stderr.trim().is_empty() {
            parts.push(format!(
                "stderr: {}",
                first_non_empty_lines(&self.stderr, 4)
            ));
        }
        if parts.is_empty() {
            format!("{} 退出码异常：{}", self.program, self.status)
        } else {
            format!(
                "{} 退出码异常：{}；{}",
                self.program,
                self.status,
                parts.join("；")
            )
        }
    }

    fn combined_text(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    fn is_access_denied(&self) -> bool {
        let text = self.combined_text();
        text_has_access_denied(&text)
    }

    fn has_successful_termination(&self) -> bool {
        let text = self.combined_text();
        let lower = text.to_ascii_lowercase();
        lower.contains("success") || text.contains("成功") || text.contains("已成功")
    }
}

#[cfg(target_os = "windows")]
fn text_has_access_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("access denied")
        || text.contains("拒绝访问")
}

#[cfg(target_os = "windows")]
fn first_non_empty_lines(text: &str, max_lines: usize) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(max_lines)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        lines.join(" / ")
    }
}

#[cfg(target_os = "windows")]
fn decode_windows_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(value) => value.trim().to_string(),
        Err(_) => {
            let (decoded, _, _) = GBK.decode(bytes);
            decoded.trim().to_string()
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let output = hidden_command(program)
        .args(args)
        .output()
        .map_err(|err| format!("执行 {program} 失败：{err}"))?;
    if !output.status.success() {
        let stderr = decode_windows_output(&output.stderr);
        return Err(if stderr.is_empty() {
            format!("{program} 退出码异常：{}", output.status)
        } else {
            stderr
        });
    }
    Ok(decode_windows_output(&output.stdout))
}

#[cfg(target_os = "windows")]
fn windows_command_status_detail(program: &str, args: &[&str]) -> Result<(), WindowsCommandError> {
    let output =
        hidden_command(program)
            .args(args)
            .output()
            .map_err(|err| WindowsCommandError {
                program: program.to_string(),
                status: "未启动".to_string(),
                stdout: String::new(),
                stderr: format!("执行 {program} 失败：{err}"),
            })?;
    if output.status.success() {
        return Ok(());
    }

    Err(WindowsCommandError {
        program: program.to_string(),
        status: output.status.to_string(),
        stdout: decode_windows_output(&output.stdout),
        stderr: decode_windows_output(&output.stderr),
    })
}

#[cfg(target_os = "windows")]
fn windows_powershell_script(script: &str) -> String {
    format!(
        r#"
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
{script}
"#
    )
}

#[cfg(target_os = "windows")]
fn windows_powershell_status(script: &str) -> Result<(), String> {
    let script = windows_powershell_script(script);
    let args = [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script.as_str(),
    ];
    windows_command_status_detail("powershell.exe", &args)
        .map_err(|err| err.detail())
        .or_else(|powershell_err| {
            windows_command_status_detail("pwsh", &args).map_err(|pwsh_err| {
                format!(
                    "powershell.exe: {powershell_err}；pwsh: {}",
                    pwsh_err.detail()
                )
            })
        })
}

#[cfg(target_os = "windows")]
fn windows_powershell_stdout(script: &str) -> Result<String, String> {
    let script = windows_powershell_script(script);
    let args = [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script.as_str(),
    ];
    windows_command_stdout("powershell.exe", &args).or_else(|powershell_err| {
        windows_command_stdout("pwsh", &args)
            .map_err(|pwsh_err| format!("powershell.exe: {powershell_err}；pwsh: {pwsh_err}"))
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn restart_chatgpt_app_impl(
    _state: &crate::store::AppState,
) -> Result<CodexAppRestartResult, String> {
    Err("当前平台暂不支持自动重启 ChatGPT/Codex App".to_string())
}

/// 重启 VS Code，令其中的 Codex 扩展重新读取本机配置与认证数据。
#[tauri::command]
pub async fn restart_vscode() -> Result<CodexAppRestartResult, String> {
    restart_vscode_impl().await
}

#[cfg(target_os = "macos")]
async fn restart_vscode_impl() -> Result<CodexAppRestartResult, String> {
    tauri::async_runtime::spawn_blocking(restart_vscode_macos)
        .await
        .map_err(|e| format!("重启 VS Code 任务失败: {e}"))?
}

#[cfg(target_os = "macos")]
fn restart_vscode_macos() -> Result<CodexAppRestartResult, String> {
    let was_running = vscode_is_running().unwrap_or(false);
    if was_running {
        if let Err(err) = quit_vscode_macos() {
            log::warn!(
                "VS Code graceful quit failed, will terminate the main process if needed: {err}"
            );
        }

        if !wait_for_vscode_macos_state(false, std::time::Duration::from_secs(8))? {
            let _ = std::process::Command::new("pkill")
                .args(["-TERM", "-x", "Code"])
                .output();
            if !wait_for_vscode_macos_state(false, std::time::Duration::from_secs(8))? {
                let _ = std::process::Command::new("pkill")
                    .args(["-KILL", "-x", "Code"])
                    .output();
                if !wait_for_vscode_macos_state(false, std::time::Duration::from_secs(4))? {
                    return Err("等待 VS Code 退出超时，请手动关闭 VS Code 后重试。".to_string());
                }
            }
        }
    }

    let output = std::process::Command::new("open")
        .args(["-b", "com.microsoft.VSCode"])
        .output()
        .map_err(|err| format!("启动 VS Code 失败: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "启动 VS Code 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    if !wait_for_vscode_macos_state(true, std::time::Duration::from_secs(15))? {
        return Err("等待 VS Code 启动超时".to_string());
    }

    Ok(CodexAppRestartResult {
        was_running,
        launched: true,
        app_path: None,
        app_id: Some("com.microsoft.VSCode".to_string()),
    })
}

#[cfg(target_os = "macos")]
fn vscode_is_running() -> Result<bool, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(r#"application id "com.microsoft.VSCode" is running"#)
        .output()
        .map_err(|err| format!("检查 VS Code 运行状态失败: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

#[cfg(target_os = "macos")]
fn quit_vscode_macos() -> Result<(), String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application id "com.microsoft.VSCode" to quit"#)
        .output()
        .map_err(|err| format!("退出 VS Code 失败: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn wait_for_vscode_macos_state(
    expected: bool,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if vscode_is_running()? == expected {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(false)
}

#[cfg(target_os = "windows")]
async fn restart_vscode_impl() -> Result<CodexAppRestartResult, String> {
    tauri::async_runtime::spawn_blocking(restart_vscode_windows)
        .await
        .map_err(|e| format!("重启 VS Code 任务失败: {e}"))?
}

#[cfg(target_os = "windows")]
fn restart_vscode_windows() -> Result<CodexAppRestartResult, String> {
    let was_running = windows_process_exists_by_image("Code.exe").unwrap_or(false);
    if was_running {
        windows_command_status_detail("taskkill", &["/F", "/T", "/IM", "Code.exe"])
            .map_err(|err| format!("退出 VS Code 失败：{}", err.detail()))?;
        if !wait_for_windows_process_state_by_image(
            "Code.exe",
            false,
            std::time::Duration::from_secs(10),
        )? {
            return Err("等待 VS Code 退出超时，请手动关闭 VS Code 后重试。".to_string());
        }
    }

    let app_path = resolve_windows_vscode_path();
    match app_path.as_deref() {
        Some(path) => hidden_command(path)
            .spawn()
            .map_err(|err| format!("启动 VS Code 失败（{}）：{err}", path.display()))?,
        None => hidden_command("code")
            .arg("--reuse-window")
            .spawn()
            .map_err(|err| {
                format!("未能定位 VS Code，请确认已安装或将 code 命令加入 PATH：{err}")
            })?,
    };

    if !wait_for_windows_process_state_by_image(
        "Code.exe",
        true,
        std::time::Duration::from_secs(15),
    )? {
        return Err("等待 VS Code 启动超时".to_string());
    }

    Ok(CodexAppRestartResult {
        was_running,
        launched: true,
        app_path: app_path.map(|path| path.to_string_lossy().to_string()),
        app_id: None,
    })
}

#[cfg(target_os = "windows")]
fn resolve_windows_vscode_path() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            std::path::PathBuf::from(local_app_data)
                .join("Programs")
                .join("Microsoft VS Code")
                .join("Code.exe"),
        );
    }
    for program_files in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(program_files) {
            candidates.push(
                std::path::PathBuf::from(root)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn restart_vscode_impl() -> Result<CodexAppRestartResult, String> {
    std::process::Command::new("code")
        .arg("--reuse-window")
        .spawn()
        .map_err(|err| format!("启动 VS Code 失败: {err}"))?;
    Ok(CodexAppRestartResult {
        was_running: false,
        launched: true,
        app_path: None,
        app_id: None,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifyHistoryMigrationResult {
    pub migrated_jsonl_files: usize,
    pub migrated_state_rows: usize,
    /// 迁移被跳过的原因（如开关关闭、live 尚未统一）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// 手动把既有官方 Codex 会话历史迁入统一 custom 桶。
#[tauri::command]
pub async fn migrate_codex_unified_history(
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<CodexUnifyHistoryMigrationResult, String> {
    crate::settings::request_codex_unify_migrate_existing().map_err(|e| e.to_string())?;
    crate::services::provider::reapply_current_codex_official_live(state.inner())
        .map_err(|e| e.to_string())?;

    let outcome = tauri::async_runtime::spawn_blocking(|| {
        crate::codex_history_migration::maybe_migrate_codex_official_history_to_unified_bucket()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    if let Some(reason) = &outcome.skipped_reason {
        log::debug!("○ Codex official history manual unify migration skipped: {reason}");
    } else {
        log::info!(
            "✓ Codex official history manual unify migration completed: jsonl_files={}, state_rows={}",
            outcome.migrated_jsonl_files,
            outcome.migrated_state_rows
        );
    }

    Ok(CodexUnifyHistoryMigrationResult {
        migrated_jsonl_files: outcome.migrated_jsonl_files,
        migrated_state_rows: outcome.migrated_state_rows,
        skipped_reason: outcome.skipped_reason,
    })
}
