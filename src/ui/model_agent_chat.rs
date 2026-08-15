use super::*;

#[component]
pub(super) fn ModelAgentChat(
    model: ModelDefinition,
    api_base_url: String,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let mut prompt = use_signal(String::new);
    let mut messages = use_signal(Vec::<AgentChatMessage>::new);
    let mut busy = use_signal(|| false);
    let model_id = model.id;
    let model_name = model.name;
    let model_title = model.title;

    rsx! {
        AgentChat {
            aria_label: "{model_title} 模型 Agent",
            messages: messages(),
            value: prompt(),
            busy: busy(),
            placeholder: "例如：增加手机号字段，并设为可查询",
            empty_text: "直接描述对 {model_title} 的修改",
            on_value_change: move |value| prompt.set(value),
            on_submit: move |request: String| {
                let request = request.trim().to_owned();
                if request.is_empty() || busy() {
                    return;
                }
                let user_message_id = format!("user-{}", messages().len());
                messages.with_mut(|items| {
                    items.push(AgentChatMessage::user(user_message_id, request.clone()));
                });
                prompt.set(String::new());
                busy.set(true);
                status.set(Some("Agent 正在修改模型".to_owned()));

                let api_base_url = api_base_url.clone();
                let model_name = model_name.clone();
                let model_title = model_title.clone();
                spawn(async move {
                    let agent_prompt = format!(
                        "只修改当前模型 {model_title}（标识: {model_name}，SymbolId: {model_id}）。\
                         不得修改其他模型、页面、函数、接口、菜单或权限。\
                         当前模型的新增、编辑、删除、字段关系与审计配置均使用 GraphPatch 完成。\
                         用户要求：{request}"
                    );
                    let run = post_api::<_, VibeRunAccepted>(
                        &api_base_url,
                        "/api/studio/program/vibe-runs",
                        &VibeRunRequest {
                            prompt: agent_prompt,
                            model: None,
                        },
                    )
                    .await;
                    let run = match run {
                        Ok(run) => run,
                        Err(error) => {
                            append_agent_result(messages, &error);
                            status.set(Some(error));
                            busy.set(false);
                            return;
                        }
                    };
                    let path = format!("/api/studio/program/vibe-runs/{}", run.session_id);
                    for _ in 0..120 {
                        crate::browser_http::sleep_ms(1_000).await;
                        match get_api::<VibeSessionSnapshot>(&api_base_url, &path).await {
                            Ok(session) if session.status == "succeeded" => {
                                generation.with_mut(|value| *value = value.saturating_add(1));
                                append_agent_result(messages, "修改已完成并发布");
                                status.set(Some("Agent 修改已完成".to_owned()));
                                busy.set(false);
                                return;
                            }
                            Ok(session) if session.status == "failed" => {
                                let failure = vibe_failure_message(&session.diagnostics);
                                append_agent_result(messages, &failure);
                                status.set(Some("Agent 修改失败".to_owned()));
                                busy.set(false);
                                return;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                append_agent_result(messages, &error);
                                status.set(Some(error));
                                busy.set(false);
                                return;
                            }
                        }
                    }
                    append_agent_result(messages, "修改仍在执行，可稍后刷新查看");
                    status.set(Some("Agent 修改超时".to_owned()));
                    busy.set(false);
                });
            },
        }
    }
}

fn append_agent_result(mut messages: Signal<Vec<AgentChatMessage>>, content: &str) {
    let message_id = format!("agent-{}", messages().len());
    messages.with_mut(|items| {
        items.push(AgentChatMessage::agent(message_id, content));
    });
}

fn vibe_failure_message(diagnostics: &Value) -> String {
    diagnostics
        .as_array()
        .and_then(|items| items.last())
        .and_then(|item| item.get("message"))
        .and_then(Value::as_str)
        .map(|message| format!("修改失败：{message}"))
        .unwrap_or_else(|| "修改失败，请调整要求后重试".to_owned())
}

#[cfg(test)]
mod tests {
    use super::vibe_failure_message;
    use serde_json::json;

    #[test]
    fn displays_latest_vibe_diagnostic() {
        let diagnostics = json!([
            {"message": "第一次失败"},
            {"message": "GraphPatch 缺少 target_id"}
        ]);

        assert_eq!(
            vibe_failure_message(&diagnostics),
            "修改失败：GraphPatch 缺少 target_id",
        );
    }

    #[test]
    fn falls_back_when_vibe_diagnostic_is_missing() {
        assert_eq!(
            vibe_failure_message(&json!([])),
            "修改失败，请调整要求后重试",
        );
    }
}
