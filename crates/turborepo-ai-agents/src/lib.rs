//! AI coding agent detection.
//!
//! Detects whether `turbo` is being invoked by an AI coding agent
//! (e.g. Claude Code, Cursor, Codex, etc.) by inspecting environment
//! variables and filesystem markers.
//!
//! Heuristics match [`@vercel/detect-agent`](https://github.com/vercel/vercel)
//! so that Turborepo and the Vercel CLI report the same agent names.

use std::{env, path::Path, sync::OnceLock};

static AGENT: OnceLock<Option<&'static str>> = OnceLock::new();

const DEVIN_LOCAL_PATH: &str = "/opt/.devin";

pub fn get_agent() -> Option<&'static str> {
    *AGENT.get_or_init(detect)
}

pub fn is_ai_agent() -> bool {
    get_agent().is_some()
}

/// Checks a single env var; returns `true` when set to a non-empty value.
fn env_is_set(name: &str) -> bool {
    !env::var(name).unwrap_or_default().is_empty()
}

/// Detection heuristics in priority order, matching `@vercel/detect-agent`.
fn detect() -> Option<&'static str> {
    // 1. Explicit self-identification — highest priority.
    if let Ok(val) = env::var("AI_AGENT") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            // Leak the trimmed string so we can return a &'static str.
            // This runs exactly once per process (OnceLock).
            return Some(Box::leak(trimmed.to_string().into_boxed_str()));
        }
    }

    // 2. Cursor (editor integrated terminal)
    if env_is_set("CURSOR_TRACE_ID") {
        return Some("cursor");
    }

    // 3. Cursor CLI agent mode
    if env_is_set("CURSOR_AGENT") {
        return Some("cursor-cli");
    }

    // 4. Google Gemini CLI
    if env_is_set("GEMINI_CLI") {
        return Some("gemini");
    }

    // 5. OpenAI Codex sandbox
    if env_is_set("CODEX_SANDBOX") {
        return Some("codex");
    }

    // 6. Augment agent
    if env_is_set("AUGMENT_AGENT") {
        return Some("augment-cli");
    }

    // 7. OpenCode
    if env_is_set("OPENCODE_CLIENT") || env_is_set("OPENCODE") {
        return Some("opencode");
    }

    // 8. Claude Code (Anthropic) — two env var spellings
    if env_is_set("CLAUDECODE") || env_is_set("CLAUDE_CODE") {
        return Some("claude");
    }

    // 9. Replit
    if env_is_set("REPL_ID") {
        return Some("replit");
    }

    // 10. Devin — detected via filesystem marker
    if Path::new(DEVIN_LOCAL_PATH).exists() {
        return Some("devin");
    }

    None
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    const ALL_AGENT_VARS: &[&str] = &[
        "AI_AGENT",
        "CURSOR_TRACE_ID",
        "CURSOR_AGENT",
        "GEMINI_CLI",
        "CODEX_SANDBOX",
        "AUGMENT_AGENT",
        "OPENCODE_CLIENT",
        "OPENCODE",
        "CLAUDECODE",
        "CLAUDE_CODE",
        "REPL_ID",
    ];

    fn clear_agent_env() {
        for var in ALL_AGENT_VARS {
            unsafe { env::remove_var(var) };
        }
    }

    #[test]
    fn test_no_agent() {
        clear_agent_env();
        assert_eq!(detect(), None);
    }

    #[test]
    fn test_ai_agent_explicit() {
        clear_agent_env();
        unsafe { env::set_var("AI_AGENT", "custom-agent") };
        assert_eq!(detect(), Some("custom-agent"));
        clear_agent_env();
    }

    #[test]
    fn test_ai_agent_whitespace_only() {
        clear_agent_env();
        unsafe { env::set_var("AI_AGENT", "   ") };
        assert_eq!(detect(), None);
        clear_agent_env();
    }

    #[test]
    fn test_ai_agent_trimmed() {
        clear_agent_env();
        unsafe { env::set_var("AI_AGENT", "  my-agent  ") };
        assert_eq!(detect(), Some("my-agent"));
        clear_agent_env();
    }

    #[test]
    fn test_cursor() {
        clear_agent_env();
        unsafe { env::set_var("CURSOR_TRACE_ID", "some-uuid") };
        assert_eq!(detect(), Some("cursor"));
        clear_agent_env();
    }

    #[test]
    fn test_cursor_cli() {
        clear_agent_env();
        unsafe { env::set_var("CURSOR_AGENT", "1") };
        assert_eq!(detect(), Some("cursor-cli"));
        clear_agent_env();
    }

    #[test]
    fn test_gemini() {
        clear_agent_env();
        unsafe { env::set_var("GEMINI_CLI", "1") };
        assert_eq!(detect(), Some("gemini"));
        clear_agent_env();
    }

    #[test]
    fn test_codex() {
        clear_agent_env();
        unsafe { env::set_var("CODEX_SANDBOX", "seatbelt") };
        assert_eq!(detect(), Some("codex"));
        clear_agent_env();
    }

    #[test]
    fn test_augment() {
        clear_agent_env();
        unsafe { env::set_var("AUGMENT_AGENT", "1") };
        assert_eq!(detect(), Some("augment-cli"));
        clear_agent_env();
    }

    #[test]
    fn test_opencode_client() {
        clear_agent_env();
        unsafe { env::set_var("OPENCODE_CLIENT", "opencode") };
        assert_eq!(detect(), Some("opencode"));
        clear_agent_env();
    }

    #[test]
    fn test_opencode() {
        clear_agent_env();
        unsafe { env::set_var("OPENCODE", "1") };
        assert_eq!(detect(), Some("opencode"));
        clear_agent_env();
    }

    #[test]
    fn test_claude_code() {
        clear_agent_env();
        unsafe { env::set_var("CLAUDE_CODE", "1") };
        assert_eq!(detect(), Some("claude"));
        clear_agent_env();
    }

    #[test]
    fn test_claudecode() {
        clear_agent_env();
        unsafe { env::set_var("CLAUDECODE", "1") };
        assert_eq!(detect(), Some("claude"));
        clear_agent_env();
    }

    #[test]
    fn test_replit() {
        clear_agent_env();
        unsafe { env::set_var("REPL_ID", "1") };
        assert_eq!(detect(), Some("replit"));
        clear_agent_env();
    }

    #[test]
    fn test_empty_env_var_ignored() {
        clear_agent_env();
        unsafe { env::set_var("CURSOR_TRACE_ID", "") };
        assert_eq!(detect(), None);
        clear_agent_env();
    }

    #[test]
    fn test_ai_agent_takes_priority() {
        clear_agent_env();
        unsafe {
            env::set_var("AI_AGENT", "custom-priority");
            env::set_var("CURSOR_TRACE_ID", "some-uuid");
            env::set_var("CLAUDE_CODE", "1");
        }
        assert_eq!(detect(), Some("custom-priority"));
        clear_agent_env();
    }

    #[test]
    fn test_cursor_takes_priority_over_claude() {
        clear_agent_env();
        unsafe {
            env::set_var("CURSOR_TRACE_ID", "some-uuid");
            env::set_var("CLAUDE_CODE", "1");
        }
        assert_eq!(detect(), Some("cursor"));
        clear_agent_env();
    }
}
