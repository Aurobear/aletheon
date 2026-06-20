use super::loader::LoadedSkill;

/// Inject loaded skills into the system prompt by appending a `[Skills]` section.
///
/// If `skills` is empty, the system prompt is returned unchanged.
pub fn inject_skills(system_prompt: &str, skills: &[LoadedSkill]) -> String {
    if skills.is_empty() {
        return system_prompt.to_string();
    }

    let mut output = String::with_capacity(system_prompt.len() + 1024);
    output.push_str(system_prompt);
    output.push_str("\n\n[Skills]\nThe following skills are available. Use them when relevant.\n");

    for skill in skills {
        output.push_str(&format!("\n## {}\n{}\n", skill.name, skill.content));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            name: name.to_string(),
            description: format!("{} description", name),
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
        assert!(result.contains("## Git Workflow"));
        assert!(result.contains("Always use feature branches."));
    }

    #[test]
    fn multiple_skills_ordered() {
        let prompt = "System prompt.";
        let skills = vec![
            make_skill("Alpha", "Alpha content."),
            make_skill("Beta", "Beta content."),
        ];
        let result = inject_skills(prompt, &skills);

        let alpha_pos = result.find("## Alpha").unwrap();
        let beta_pos = result.find("## Beta").unwrap();
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
