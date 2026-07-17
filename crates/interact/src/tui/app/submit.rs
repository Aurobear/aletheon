use std::io;
use std::io::Write;

use fabric::protocol::client::ClientRpcRequest;
use fabric::ui_event::CollaborationMode;
use fabric::ui_event::InterruptReason;
use tokio::io::AsyncWriteExt;

use super::super::chat::{ChatWidget, Role as ChatRole};
use super::super::command::{looks_like_command, parse_command, BuiltinCommand, CommandType};
use super::super::App;

pub(super) async fn write_request(app: &mut App, request: ClientRpcRequest) {
    let request = request
        .to_json_rpc(Some(1))
        .expect("typed client request serializes");
    let payload = serde_json::to_string(&request).unwrap_or_default();
    let framed = format!("{}\n", payload);
    let _ = app.stream.write_all(framed.as_bytes()).await;
    let _ = app.stream.flush().await;
}

/// Send a typed protocol request whose response is handled by the streaming
/// response path.
async fn send_request(app: &mut App, request: ClientRpcRequest) {
    write_request(app, request).await;
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
}

pub async fn submit_message(app: &mut App, text: String) {
    // Check for /commands (but NOT absolute paths like /home/... — those are chat)
    if looks_like_command(&text) {
        let parsed = parse_command(&text);
        match parsed {
            Some(CommandType::Builtin(BuiltinCommand::Quit)) => {
                app.running = false;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Clear)) => {
                app.chat = ChatWidget::new(app.caps.clone());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Copy)) => {
                // Copy last assistant message to clipboard via OSC 52
                let last_assistant = app.chat.entries.iter().rev().find_map(|entry| {
                    if let super::super::chat::ChatEntry::Text(ref msg) = entry {
                        if msg.role == ChatRole::Assistant {
                            Some(msg.content.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });
                match last_assistant {
                    Some(text) if !text.is_empty() => {
                        let encoded = base64_encode(&text);
                        // OSC 52: set clipboard to base64-encoded text
                        let osc = format!("\x1b]52;c;{}\x1b\\", encoded);
                        io::stdout().write_all(osc.as_bytes()).ok();
                        io::stdout().flush().ok();
                        app.chat
                            .add_text(ChatRole::System, "已复制到剪贴板".to_string());
                    }
                    _ => {
                        app.chat
                            .add_text(ChatRole::System, "没有可复制的内容".to_string());
                    }
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Help)) => {
                let help = "内置命令：\n  /help         显示帮助\n  /clear        清空对话\n  /copy         复制最后回复到剪贴板\n  /status (st)  查看自我演化状态\n  /reflect      查看反思记录\n  /reflect_now  执行即时反思\n  /evolution    查看演化历史\n  /genome       查看基因组\n  /sessions     列出会话\n  /resume <id>  恢复会话\n  /compact (cmp) 压缩上下文\n  /model (m)    切换模型\n  /quit         退出\n\n输入：\n  Shift+Enter 或 \\+Enter  换行\n  Enter                   发送\n  Ctrl+C                   清空/退出\n  Esc                      清空输入\n  PgUp/PgDn               滚动聊天";
                app.chat.add_text(ChatRole::System, help.to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Status)) => {
                send_request(app, ClientRpcRequest::Status).await;
                app.chat
                    .add_text(ChatRole::System, "查询状态中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Reflect)) => {
                send_request(app, ClientRpcRequest::Reflect).await;
                app.chat
                    .add_text(ChatRole::System, "查询反思记录中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::ReflectNow)) => {
                send_request(app, ClientRpcRequest::ReflectNow).await;
                app.chat
                    .add_text(ChatRole::System, "执行即时反思中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Evolution)) => {
                send_request(app, ClientRpcRequest::Evolution).await;
                app.chat
                    .add_text(ChatRole::System, "查询演化历史中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Genome)) => {
                send_request(app, ClientRpcRequest::Genome).await;
                app.chat
                    .add_text(ChatRole::System, "查询基因组中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Sessions)) => {
                send_request(app, ClientRpcRequest::Sessions).await;
                app.chat
                    .add_text(ChatRole::System, "查询会话列表中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Resume { id })) => {
                if id.is_empty() {
                    app.chat
                        .add_text(ChatRole::System, "用法: /resume <session_id>".to_string());
                    return;
                }
                send_request(app, ClientRpcRequest::resume(id.clone())).await;
                app.chat
                    .add_text(ChatRole::System, format!("恢复会话 {}...", id));
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Compact)) => {
                send_request(app, ClientRpcRequest::Compact).await;
                app.chat
                    .add_text(ChatRole::System, "压缩上下文中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Model)) => {
                send_request(app, ClientRpcRequest::ModelList).await;
                app.chat
                    .add_text(ChatRole::System, "查询可用模型中...".to_string());
                return;
            }
            // ── New P2 commands ──
            Some(CommandType::Builtin(BuiltinCommand::Mode { name })) => {
                let mode = if name.is_empty() {
                    // Cycle to next mode
                    let modes = [
                        CollaborationMode::Default,
                        CollaborationMode::Plan,
                        CollaborationMode::Auto,
                        CollaborationMode::Sandbox,
                    ];
                    let current = modes
                        .iter()
                        .position(|m| *m == app.app_state.mode)
                        .unwrap_or(0);
                    let next = modes[(current + 1) % modes.len()];
                    next
                } else {
                    match name.as_str() {
                        "plan" => CollaborationMode::Plan,
                        "auto" => CollaborationMode::Auto,
                        "sandbox" => CollaborationMode::Sandbox,
                        _ => CollaborationMode::Default,
                    }
                };
                write_request(app, ClientRpcRequest::mode_switch(mode)).await;
                app.chat.add_text(
                    ChatRole::System,
                    format!("Switching mode to: {}", mode.display_name()),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Plan)) => {
                let target = if app.app_state.mode == CollaborationMode::Plan {
                    CollaborationMode::Default
                } else {
                    CollaborationMode::Plan
                };
                write_request(app, ClientRpcRequest::mode_switch(target)).await;
                app.chat.add_text(
                    ChatRole::System,
                    format!("Switching to {} mode", target.display_name()),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Approve)) => {
                write_request(app, ClientRpcRequest::PlanApprove).await;
                app.chat
                    .add_text(ChatRole::System, "Plan approved".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Agents)) => {
                if app.sub_agents.is_empty() {
                    app.chat
                        .add_text(ChatRole::System, "No active sub-agents".to_string());
                } else {
                    let lines: Vec<String> = app
                        .sub_agents
                        .iter()
                        .map(|a| format!("  {} - {:?}: {}", a.id, a.status, a.task))
                        .collect();
                    app.chat.add_text(
                        ChatRole::System,
                        format!("Active sub-agents:\n{}", lines.join("\n")),
                    );
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::AgentDetail { id })) => {
                if let Some(agent) = app.sub_agents.iter().find(|a| a.id == id) {
                    let msg = format!(
                        "Agent: {}\nTask: {}\nStatus: {:?}\nParent: {}",
                        agent.id, agent.task, agent.status, agent.parent_turn_id
                    );
                    app.chat.add_text(ChatRole::System, msg);
                } else {
                    app.chat
                        .add_text(ChatRole::System, format!("Agent not found: {}", id));
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Hooks)) => {
                send_request(app, ClientRpcRequest::HooksList).await;
                app.chat
                    .add_text(ChatRole::System, "Querying hooks...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Skills)) => {
                let skills = app.skill_loader.list();
                if skills.is_empty() {
                    app.chat
                        .add_text(ChatRole::System, "No skills loaded".to_string());
                } else {
                    let lines: Vec<String> = skills
                        .iter()
                        .map(|s| format!("  /{} - {}", s.name, s.description))
                        .collect();
                    app.chat.add_text(
                        ChatRole::System,
                        format!("Available skills:\n{}", lines.join("\n")),
                    );
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::SkillRun { name, args })) => {
                let skill = match app.skill_loader.get(&name) {
                    Some(s) => s.clone(),
                    None => {
                        app.chat
                            .add_text(ChatRole::System, format!("Unknown skill: /{}", name));
                        return;
                    }
                };
                let message = if args.is_empty() {
                    skill.content.clone()
                } else {
                    format!("{}\n\nUser input: {}", skill.content, args)
                };
                app.chat.add_text(ChatRole::User, text.clone());
                // Assistant entry created lazily on first response delta so it
                // renders after any tool/reflection logs (ordering fix).
                send_to_daemon(app, &message).await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Interrupt)) => {
                write_request(
                    app,
                    ClientRpcRequest::interrupt(InterruptReason::UserCancelled),
                )
                .await;
                app.chat
                    .add_text(ChatRole::System, "Interrupt sent".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Context)) => {
                let ctx = &app.app_state.context;
                let mode = &app.app_state.mode;
                let msg = format!(
                    "Context: {}\nMode: {} {}\nModel: {}\nTokens: {}k\nAwareness: {} {}",
                    ctx.display(),
                    mode.icon(),
                    mode.display_name(),
                    app.app_state.model_name,
                    app.app_state.total_tokens / 1000,
                    app.app_state.awareness.level.icon(),
                    app.app_state.awareness.level.display_name(),
                );
                app.chat.add_text(ChatRole::System, msg);
                return;
            }
            Some(CommandType::Builtin(_)) => return,
            Some(CommandType::Skill { name, args }) => {
                app.chat.add_text(ChatRole::User, text.clone());
                let skill = match app.skill_loader.get(&name) {
                    Some(s) => s.clone(),
                    None => {
                        app.chat
                            .add_text(ChatRole::System, format!("未知技能: /{}", name));
                        return;
                    }
                };
                let message = if args.is_empty() {
                    skill.content.clone()
                } else {
                    format!("{}\n\nUser input: {}", skill.content, args)
                };
                // Assistant entry created lazily on first response delta so it
                // renders after any tool/reflection logs (ordering fix).
                send_to_daemon(app, &message).await;
                return;
            }
            None => {
                app.chat.add_text(ChatRole::System, "无效命令".to_string());
                return;
            }
        }
    }

    // Regular chat message
    app.history.push(text.clone());
    app.chat.add_text(ChatRole::User, text.clone());
    // Assistant entry created lazily on first response delta so it renders
    // after any tool/reflection logs (ordering fix).
    send_to_daemon(app, &text).await;
}

pub async fn send_to_daemon(app: &mut App, text: &str) {
    let msg = crate::tui::chat_request(text, &app.workspace);
    let payload = serde_json::to_string(&msg).unwrap_or_default();
    let framed = format!("{}\n", payload);

    if app.stream.write_all(framed.as_bytes()).await.is_err() {
        app.chat
            .add_text(ChatRole::System, "发送失败，请检查 daemon".to_string());
        return;
    }
    let _ = app.stream.flush().await;
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
    app.app_state.streaming = true;
}

/// Simple base64 encoder (no external dependency).
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).map(|&b| b as u32).unwrap_or(0);
        let b2 = chunk.get(2).map(|&b| b as u32).unwrap_or(0);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
