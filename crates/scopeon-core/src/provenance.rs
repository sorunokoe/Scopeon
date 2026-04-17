use std::collections::{HashMap, HashSet};

use crate::{InteractionEvent, ProviderCapability};

pub fn provider_capabilities(provider: &str) -> Vec<ProviderCapability> {
    let capabilities = match provider {
        "copilot-cli" => vec![
            ("skills", "estimated", "Skill usage is emitted directly; token impact is length-based."),
            ("hooks", "estimated", "Hook lifecycle is emitted directly; intervention effect is derived from later tool execution."),
            ("mcp_identity", "exact", "MCP server and tool names are emitted directly."),
            ("task_history", "exact", "Task lifecycle and subagent totals are emitted directly."),
            ("interaction_tokens", "estimated", "Most interaction token costs are inferred from safe sizes; subagent totals can be exact."),
        ],
        "claude-code" => vec![
            ("skills", "unsupported", "Claude session logs do not expose skill lifecycle events."),
            ("hooks", "unsupported", "Claude session logs do not expose hook lifecycle events."),
            ("mcp_identity", "exact", "MCP tool names are embedded in tool_use names."),
            ("task_history", "estimated", "Task tool usage can be observed, but full task lifecycle is not exposed."),
            ("interaction_tokens", "estimated", "Turn totals are exact, but per-interaction attribution is inferred."),
        ],
        "gemini-cli" | "aider" | "generic-openai" => vec![
            ("skills", "unsupported", "Provider logs do not expose skill lifecycle events."),
            ("hooks", "unsupported", "Provider logs do not expose hook lifecycle events."),
            ("mcp_identity", "estimated", "Some tool usage may be visible, but MCP identity is not guaranteed."),
            ("task_history", "unsupported", "Provider logs do not expose task lifecycle events."),
            ("interaction_tokens", "estimated", "Only coarse turn-level attribution is available."),
        ],
        "ollama" => vec![
            ("skills", "unsupported", "Ollama chats do not expose agent skill lifecycle."),
            ("hooks", "unsupported", "Ollama chats do not expose hook lifecycle."),
            ("mcp_identity", "unsupported", "Ollama chat history does not include MCP metadata."),
            ("task_history", "unsupported", "Ollama chat history does not include task lifecycle."),
            ("interaction_tokens", "estimated", "Only coarse output length approximations are available."),
        ],
        _ => vec![
            ("skills", "unsupported", "No provider-specific provenance adapter is available."),
            ("hooks", "unsupported", "No provider-specific provenance adapter is available."),
            ("mcp_identity", "unsupported", "No provider-specific provenance adapter is available."),
            ("task_history", "unsupported", "No provider-specific provenance adapter is available."),
            ("interaction_tokens", "unsupported", "No provider-specific provenance adapter is available."),
        ],
    };

    capabilities
        .into_iter()
        .map(|(capability, level, note)| ProviderCapability {
            provider: provider.to_string(),
            capability: capability.to_string(),
            level: level.to_string(),
            note: note.to_string(),
        })
        .collect()
}

pub fn derive_hook_effects(events: &[InteractionEvent]) -> HashMap<String, String> {
    let mut tool_inputs: HashMap<&str, i64> = HashMap::new();
    let mut hook_success: HashMap<&str, bool> = HashMap::new();
    let mut hook_effects = HashMap::new();
    let mut executed_tool_ids = HashSet::new();

    for event in events {
        if matches!(event.kind.as_str(), "tool" | "mcp" | "task" | "skill")
            && event.phase == "start"
            && event.correlation_id.is_some()
        {
            if let Some(tool_id) = event.correlation_id.as_deref() {
                tool_inputs.insert(tool_id, event.input_size_chars);
                executed_tool_ids.insert(tool_id.to_string());
            }
        }
        if event.kind == "hook" && event.phase == "complete" {
            if let Some(parent_id) = event.parent_id.as_deref() {
                hook_success.insert(parent_id, event.success.unwrap_or(true));
            }
        }
    }

    for event in events {
        if event.kind != "hook" || event.phase != "start" {
            continue;
        }

        let effect = if let Some(tool_id) = event.correlation_id.as_deref() {
            if let Some(tool_input) = tool_inputs.get(tool_id) {
                if *tool_input == event.input_size_chars {
                    "pass_through"
                } else {
                    "modified"
                }
            } else {
                "blocked"
            }
        } else if let Some(parent_id) = event.parent_id.as_deref() {
            if hook_success.get(parent_id).copied().unwrap_or(true) {
                "pass_through"
            } else {
                "blocked"
            }
        } else {
            "observed"
        };

        hook_effects.insert(event.id.clone(), effect.to_string());
    }

    hook_effects
}

pub fn interaction_token_total(event: &InteractionEvent) -> i64 {
    event
        .total_tokens
        .unwrap_or(event.estimated_input_tokens + event.estimated_output_tokens)
}
