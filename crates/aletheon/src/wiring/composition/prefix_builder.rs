use corpus::skill::loader::LoadedSkill;

/// Builds a deterministic, cache-stable system prompt prefix.
///
/// The prefix is assembled once at boot and never mutated mid-session.
/// Selected memory arrives later as untrusted conscious-context data, so the
/// prefix stays byte-stable across turns for maximum cache reuse.
pub struct PrefixBuilder;

impl PrefixBuilder {
    /// Build the prefix from its components.
    /// Order is deterministic: base -> skills. Memory is selected through the
    /// conscious workspace and never enters the system prefix directly.
    /// Same inputs always produce the same bytes.
    pub fn build(config_prompt: &str, skills: &[LoadedSkill]) -> String {
        let mut prefix = String::with_capacity(4096);

        // 1. Base system prompt (most stable text — stays as cache prefix)
        prefix.push_str(config_prompt);

        // Skills are selected per turn by the skill router. Injecting the full
        // catalog here wastes stable-prefix tokens and exposes irrelevant
        // descriptions before the user asks for them.
        let _ = skills;

        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::skill::loader::LoadedSkill;

    fn make_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            name: name.to_string(),
            description: format!("{name} desc"),
            content: content.to_string(),
            source: "test".to_string(),
        }
    }

    #[test]
    fn deterministic_same_inputs() {
        let skills = vec![make_skill("git", "Use feature branches.")];
        let p1 = PrefixBuilder::build("You are Aletheon.", &skills);
        let p2 = PrefixBuilder::build("You are Aletheon.", &skills);
        assert_eq!(p1, p2);
    }

    #[test]
    fn different_base_changes_prefix() {
        let p1 = PrefixBuilder::build("Prompt A.", &[]);
        let p2 = PrefixBuilder::build("Prompt B.", &[]);
        assert_ne!(p1, p2);
    }

    #[test]
    fn skills_are_not_injected_into_the_stable_prefix() {
        let skills = vec![make_skill("test", "content")];
        let prefix = PrefixBuilder::build("Base.", &skills);
        assert_eq!(prefix, "Base.");
    }

    #[test]
    fn core_memory_never_enters_system_prefix() {
        let prefix = PrefixBuilder::build("Base.", &[]);
        assert!(!prefix.contains("[Persona]"));
        assert!(!prefix.contains("<memory"));
    }

    #[test]
    fn skills_index_does_not_expose_description_or_full_body() {
        let skill = LoadedSkill {
            name: "large".into(),
            description: "Short routing summary".into(),
            content: "FULL_BODY_SHOULD_NOT_BE_IN_PREFIX".repeat(10_000),
            source: "test".into(),
        };

        let prefix = PrefixBuilder::build("Base.", &[skill]);

        assert!(!prefix.contains("Short routing summary"));
        assert!(!prefix.contains("FULL_BODY_SHOULD_NOT_BE_IN_PREFIX"));
        assert_eq!(prefix, "Base.");
    }

    #[test]
    fn many_skills_do_not_change_prefix_size() {
        let skills = (0..100)
            .map(|i| LoadedSkill {
                name: format!("skill-{i}"),
                description: "d".repeat(2_000),
                content: String::new(),
                source: "test".into(),
            })
            .collect::<Vec<_>>();

        let prefix = PrefixBuilder::build("Base.", &skills);

        assert_eq!(prefix, "Base.");
    }
}
