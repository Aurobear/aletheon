//! Keyword-triggered skill activation.

#[derive(Debug, Clone)]
pub struct SkillKeywords {
    pub name: String,
    pub keywords: Vec<String>,
    pub body: String,
}

/// Maximum number of matched skill bodies returned by `match_skills`, to
/// avoid concatenating too many full skill bodies for casual chat.
const MAX_MATCHED_SKILLS: usize = 3;

/// Return the bodies of the top-matching skills whose any keyword appears in
/// `message`, ranked by number of distinct keyword hits (descending) and
/// capped at `MAX_MATCHED_SKILLS`.
pub fn match_skills(message: &str, skills: &[SkillKeywords]) -> Vec<String> {
    let lower = message.to_lowercase();
    let mut matched: Vec<(usize, &SkillKeywords)> = skills
        .iter()
        .filter_map(|s| {
            let hits = s
                .keywords
                .iter()
                .filter(|k| !k.is_empty() && lower.contains(&k.to_lowercase()))
                .count();
            (hits > 0).then_some((hits, s))
        })
        .collect();

    matched.sort_by(|a, b| b.0.cmp(&a.0));

    matched
        .into_iter()
        .take(MAX_MATCHED_SKILLS)
        .map(|(_, s)| s.body.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_skill_by_keyword() {
        let skills = vec![SkillKeywords {
            name: "git".into(),
            keywords: vec!["git".into(), "commit".into(), "branch".into()],
            body: "Always use feature branches.".into(),
        }];

        let matched = match_skills("please commit my branch", &skills);
        assert_eq!(matched.len(), 1);
        assert!(matched[0].contains("feature branches"));
    }

    #[test]
    fn no_keyword_no_match() {
        let skills = vec![SkillKeywords {
            name: "git".into(),
            keywords: vec!["commit".into(), "branch".into()],
            body: "Always use feature branches.".into(),
        }];

        let matched = match_skills("hello world", &skills);
        assert!(matched.is_empty());
    }

    #[test]
    fn case_insensitive_matching() {
        let skills = vec![SkillKeywords {
            name: "git".into(),
            keywords: vec!["Git".into()],
            body: "Git skill body.".into(),
        }];

        let matched = match_skills("use GIT for version control", &skills);
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn empty_keyword_skipped() {
        let skills = vec![SkillKeywords {
            name: "empty".into(),
            keywords: vec!["".into()],
            body: "Should not match.".into(),
        }];

        let matched = match_skills("anything", &skills);
        assert!(matched.is_empty());
    }

    #[test]
    fn multiple_skills_matched() {
        let skills = vec![
            SkillKeywords {
                name: "git".into(),
                keywords: vec!["commit".into()],
                body: "Git body.".into(),
            },
            SkillKeywords {
                name: "deploy".into(),
                keywords: vec!["deploy".into()],
                body: "Deploy body.".into(),
            },
            SkillKeywords {
                name: "unrelated".into(),
                keywords: vec!["unrelated".into()],
                body: "No match.".into(),
            },
        ];

        let matched = match_skills("commit and deploy", &skills);
        assert_eq!(matched.len(), 2);
    }
}
