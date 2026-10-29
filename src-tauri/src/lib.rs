//! AiClub 后端装配。
//!
//! 模块边界(每块都能单独读懂):
//! - `config`    配置模型与持久化
//! - `sandbox`   路径守卫(纯函数,最严测试)
//! - `provider`  OpenAI 兼容流式 + 工具调用增量 + SSE(每方独立 client)
//! - `validate`  独立性硬闸(D-011)
//! - `tools`     16 个工具 schema + dispatch
//! - `shell`     受限执行器(限时、限输出、剥密钥、杀进程树)
//! - `skills`    共享只读技能库
//! - `memory`    按方隔离的私有记忆
//! - `transcript`上下文重建(对方的动作只给一行摘要)
//! - `engine`    轮次状态机(只依赖 trait,测试不需要网络)
//! - `commands`  Tauri 命令(没有通往对话的写入通道)

pub mod commands;
pub mod config;
pub mod connection;
pub mod engine;
pub mod events;
pub mod memory;
pub mod network;
pub mod provider;
pub mod sandbox;
pub mod shell;
pub mod skills;
pub mod tools;
pub mod transcript;
pub mod validate;

/// 测试辅助。只在 `cargo test` 下编译,不进产物。
#[cfg(test)]
pub mod testsup;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::RunningState::default());

    builder
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_config,
            commands::save_config,
            commands::check_pair,
            commands::detect,
            connection::test_connection,
            commands::touch_workspace,
            commands::start_session,
            commands::status,
            commands::pause,
            commands::stop_session,
            commands::events,
            commands::events_of,
            commands::sessions,
            commands::delete_session,
            commands::export_session,
            commands::tree,
            commands::read_workspace,
            commands::memories,
            commands::skills_list,
        ])
        .setup(|app| {
            // 首次运行:落一份默认配置,保证 config.toml 存在。
            // 写失败不 panic:界面会用内存里的默认值继续工作,设置页可重存。
            let exists = config::config_path()
                .map(|p| std::path::Path::new(&p).exists())
                .unwrap_or(false);
            if !exists {
                let _ = config::save(&config::AppConfig::default());
            }
            let _ = app.handle().clone();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("AiClub 启动失败");
}
