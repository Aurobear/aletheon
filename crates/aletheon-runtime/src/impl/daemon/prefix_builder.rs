use crate::r#impl::skills::loader::LoadedSkill;
use crate::CoreMemory;

/// Builds a deterministic, cache-stable system prompt prefix.
///
/// The prefix is assembled once at boot and never mutated mid-session.
/// Memory changes ride user turns as `<memory-update>` XML blocks,
/// so the prefix stays byte-stable across turns for maximum cache reuse.
pub struct PrefixBuilder;

impl PrefixBuilder {
    /// Build the prefix from its components.
    /// Order is deterministic: base -> skills -> core memory.
    /// Same inputs always produce the same bytes.
    pub fn build(
        config_prompt: &str,
        skills: &[LoadedSkill],
        core_memory: &CoreMemory,
    ) -> String {
        let mut prefix = String::with_capacity(4096);

        // 1. Base system prompt (most stable text — stays as cache prefix)
        prefix.push_str(config_prompt);

        // 2. Skills index (names + descriptions only, not full content)
        if !skills.is_empty() {
            prefix.push_str(
                "\n\n[Skills]\nThe following skills are available. Use them when relevant.\n",
            );
            for skill in skills {
                prefix.push_str(&format!("\n## {}\n{}\n", skill.name, skill.content));
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
    use crate::r#impl::skills::loader::LoadedSkill;

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
    fn diff_reason_detects_change() {
        let reason = PrefixBuilder::diff_reason("hello", "hello!");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("length changed"));
    }

    #[test]
    fn diff_reason_none_for_identical() {
        assert!(PrefixBuilder::diff_reason("same", "same").is_none());
    }
}
