use super::loader::LoadedSkill;

/// Inject loaded skills into the system prompt as a compact catalog (name +
/// description only). The full instructions of a skill are loaded on demand by
/// the model via the `skill_get` tool, so the system prompt stays small even
/// with many skills. If `skills` is empty, the prompt is returned unchanged.
pub fn inject_skills(system_prompt: &str, skills: &[LoadedSkill]) -> String {
    if skills.is_empty() {
        return system_prompt.to_string();
    }

    let mut output = String::with_capacity(system_prompt.len() + 512);
    output.push_str(system_prompt);
    output.push_str(
        "\n\n[Skills]\nThe following skills are available. When a task matches one, \
         call the `skill_get` tool with its name to load the full instructions \
         before proceeding.\n",
    );

    for skill in skills {
        output.push_str(&format!("\n- {}: {}", skill.name, skill.description));
    }
    output.push('\n');

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            name: name.to_string(),
            description: format!("{name} description"),
            content: content.to_string(),
            source: "system".to_string(),
        }
    }

    #[test]
    fn empty_skills_returns_original_prompt() {
        let prompt = "You are Aletheon.";
        let result = inject_skills(prompt, &[]);
        assert_eq!(result, prompt);
    }

    #[test]
    fn single_skill_appended() {
        let prompt = "You are Aletheon.";
        let skills = vec![make_skill("Git Workflow", "Always use feature branches.")];
        let result = inject_skills(prompt, &skills);

        assert!(result.starts_with(prompt));
        assert!(result.contains("[Skills]"));
        // Catalog lists name + description and points at skill_get; the full
        // content is NOT injected (it is loaded on demand via skill_get).
        assert!(result.contains("- Git Workflow: Git Workflow description"));
        assert!(result.contains("skill_get"));
        assert!(!result.contains("Always use feature branches."));
    }

    #[test]
    fn multiple_skills_ordered() {
        let prompt = "System prompt.";
        let skills = vec![
            make_skill("Alpha", "Alpha content."),
            make_skill("Beta", "Beta content."),
        ];
        let result = inject_skills(prompt, &skills);

        let alpha_pos = result.find("- Alpha:").unwrap();
        let beta_pos = result.find("- Beta:").unwrap();
        assert!(alpha_pos < beta_pos);
    }

    #[test]
    fn skills_section_header_present() {
        let prompt = "Base.";
        let skills = vec![make_skill("Test", "Content.")];
        let result = inject_skills(prompt, &skills);

        assert!(result.contains("[Skills]"));
        assert!(result.contains("The following skills are available."));
    }
}
