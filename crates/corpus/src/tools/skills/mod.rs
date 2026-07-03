//! Skills system -- two-layer architecture.
//!
//! Layer 1: Built-in skills (Rust code) -- registered at startup.
//! Layer 2: User skills (Markdown prompts) -- loaded from ~/.aletheon/skills/.

pub mod loader;
pub mod markdown_skill;

pub use loader::SkillLoader;
pub use markdown_skill::MarkdownSkill;
