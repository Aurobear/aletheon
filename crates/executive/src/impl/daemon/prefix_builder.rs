use corpus::skill::loader::LoadedSkill;
use mnemosyne::CoreMemory;

/// Builds a deterministic, cache-stable system prompt prefix.
///
/// The prefix is assembled once at boot and never mutated mid-session.
/// Memory changes ride user turns as `<memory-update>` XML blocks,
/// so the prefix stays byte-stable across turns for maximum cache reuse.
pub struct PrefixBuilder;

const MAX_SKILL_DESCRIPTION_CHARS: usize = 512;
const MAX_SKILLS_INDEX_CHARS: usize = 16 * 1024;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let truncated: String = value.chars().take(max_chars - 1).collect();
    format!("{truncated}…")
}

impl PrefixBuilder {
    /// Build the prefix from its components.
    /// Order is deterministic: base -> skills -> core memory.
    /// Same inputs always produce the same bytes.
    pub fn build(config_prompt: &str, skills: &[LoadedSkill], core_memory: &CoreMemory) -> String {
        let mut prefix = String::with_capacity(4096);

        // 1. Base system prompt (most stable text — stays as cache prefix)
        prefix.push_str(config_prompt);

        // 2. Skills index (names + descriptions only, not full content)
        if !skills.is_empty() {
            prefix.push_str(
                "\n\n[Skills]\nThe following skills are available. Use them when relevant.\n",
            );
            let mut index_chars = 0;
            for skill in skills {
                let description = truncate_chars(&skill.description, MAX_SKILL_DESCRIPTION_CHARS);
                let entry = format!("\n## {}\n{}\n", skill.name, description);
                let remaining = MAX_SKILLS_INDEX_CHARS.saturating_sub(index_chars);
                if remaining == 0 {
                    break;
                }
                let bounded = truncate_chars(&entry, remaining);
                index_chars += bounded.chars().count();
                prefix.push_str(&bounded);
            }
        }

        // 3. Core memory blocks (snapshot at boot time)
        let memory_text = core_memory.inject_into_prompt();
        if !memory_text.is_empty() {
            prefix.push_str("\n\n");
            prefix.push_str(&memory_text);
        }

        prefix
    }

    /// Build the prefix with optional DaseinContext injection.
    ///
    /// When `dasein_context` is `Some`, the formatted existential state is
    /// appended after core memory. This gives the LLM awareness of the
    /// system's mood, temporal flow, involvement network, and care structure.
    ///
    /// Note: DaseinContext is intentionally appended LAST (after core memory)
    /// because it changes more frequently than skills or core memory.
    /// If cache stability is critical, callers should inject dasein context
    /// into the user message instead via `<dasein-state>` blocks.
    pub fn build_with_dasein(
        config_prompt: &str,
        skills: &[LoadedSkill],
        core_memory: &CoreMemory,
        dasein_context: Option<&str>,
    ) -> String {
        let mut prefix = Self::build(config_prompt, skills, core_memory);

        if let Some(ctx) = dasein_context {
            if !ctx.is_empty() {
                prefix.push_str("\n\n");
                prefix.push_str(ctx);
            }
        }

        prefix
    }

    /// Compare two prefixes and return whether they differ.
    /// Useful for diagnostics: explains why a cache miss happened.
    pub fn diff_reason(old: &str, new: &str) -> Option<String> {
        if old == new {
            return None;
        }
        if old.len() != new.len() {
            return Some(format!(
                "prefix length changed: {} -> {} bytes",
                old.len(),
                new.len()
            ));
        }
        // Find first differing byte
        for (i, (a, b)) in old.bytes().zip(new.bytes()).enumerate() {
            if a != b {
                return Some(format!(
                    "prefix differs at byte {}: {:02x} -> {:02x}",
                    i, a, b
                ));
            }
        }
        Some("prefix differs (unknown reason)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::skill::loader::LoadedSkill;

    fn make_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            name: name.to_string(),
            description: format!("{} desc", name),
            content: content.to_string(),
            source: "test".to_string(),
        }
    }

    #[test]
    fn deterministic_same_inputs() {
        let mem = CoreMemory::with_defaults();
        let skills = vec![make_skill("git", "Use feature branches.")];
        let p1 = PrefixBuilder::build("You are Aletheon.", &skills, &mem);
        let p2 = PrefixBuilder::build("You are Aletheon.", &skills, &mem);
        assert_eq!(p1, p2);
    }

    #[test]
    fn different_base_changes_prefix() {
        let mem = CoreMemory::with_defaults();
        let p1 = PrefixBuilder::build("Prompt A.", &[], &mem);
        let p2 = PrefixBuilder::build("Prompt B.", &[], &mem);
        assert_ne!(p1, p2);
    }

    #[test]
    fn skills_appended_after_base() {
        let mem = CoreMemory::with_defaults();
        let skills = vec![make_skill("test", "content")];
        let prefix = PrefixBuilder::build("Base.", &skills, &mem);
        let base_pos = prefix.find("Base.").unwrap();
        let skills_pos = prefix.find("[Skills]").unwrap();
        assert!(base_pos < skills_pos);
    }

    #[test]
    fn core_memory_after_skills() {
        let mut mem = CoreMemory::with_defaults();
        mem.append("system_state", "running").unwrap();
        let skills = vec![make_skill("s", "c")];
        let prefix = PrefixBuilder::build("Base.", &skills, &mem);
        let skills_pos = prefix.find("[Skills]").unwrap();
        let memory_pos = prefix.find("[Persona]").unwrap();
        assert!(skills_pos < memory_pos);
    }

    #[test]
    fn skills_index_uses_description_not_full_body() {
        let mem = CoreMemory::with_defaults();
        let skill = LoadedSkill {
            name: "large".into(),
            description: "Short routing summary".into(),
            content: "FULL_BODY_SHOULD_NOT_BE_IN_PREFIX".repeat(10_000),
            source: "test".into(),
        };

        let prefix = PrefixBuilder::build("Base.", &[skill], &mem);

        assert!(prefix.contains("Short routing summary"));
        assert!(!prefix.contains("FULL_BODY_SHOULD_NOT_BE_IN_PREFIX"));
        assert!(prefix.len() < 32 * 1024);
    }

    #[test]
    fn skills_index_has_total_budget() {
        let mem = CoreMemory::with_defaults();
        let skills = (0..100)
            .map(|i| LoadedSkill {
                name: format!("skill-{i}"),
                description: "d".repeat(2_000),
                content: String::new(),
                source: "test".into(),
            })
            .collect::<Vec<_>>();

        let prefix = PrefixBuilder::build("Base.", &skills, &mem);

        assert!(prefix.len() < 32 * 1024);
    }

    #[test]
    fn diff_reason_detects_change() {
        let reason = PrefixBuilder::diff_reason("hello", "hello!");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("length changed"));
    }

    #[test]
    fn diff_reason_none_for_identical() {
        assert!(PrefixBuilder::diff_reason("same", "same").is_none());
    }

    #[test]
    fn build_with_dasein_appends_context() {
        let mem = CoreMemory::with_defaults();
        let dasein = "## Existential State\nMood: calm";
        let prefix = PrefixBuilder::build_with_dasein("Base.", &[], &mem, Some(dasein));
        assert!(prefix.contains("Base."));
        assert!(prefix.contains("## Existential State"));
        assert!(prefix.contains("Mood: calm"));
    }

    #[test]
    fn build_with_dasein_none_is_same_as_build() {
        let mem = CoreMemory::with_defaults();
        let skills = vec![make_skill("test", "content")];
        let p1 = PrefixBuilder::build("Base.", &skills, &mem);
        let p2 = PrefixBuilder::build_with_dasein("Base.", &skills, &mem, None);
        assert_eq!(p1, p2);
    }

    #[test]
    fn build_with_dasein_empty_is_same_as_build() {
        let mem = CoreMemory::with_defaults();
        let p1 = PrefixBuilder::build("Base.", &[], &mem);
        let p2 = PrefixBuilder::build_with_dasein("Base.", &[], &mem, Some(""));
        assert_eq!(p1, p2);
    }
}
