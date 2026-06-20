//! Skills system -- two-layer architecture.
//!
//! Layer 1: Built-in skills (Rust code) -- registered at startup.
//! Layer 2: User skills (Markdown prompts) -- loaded from ~/.aletheon/skills/.

pub mod markdown_skill;
pub mod loader;

pub use markdown_skill::MarkdownSkill;
pub use loader::SkillLoader;
