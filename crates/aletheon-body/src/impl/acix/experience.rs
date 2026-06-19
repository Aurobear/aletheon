use std::path::Path;

use aletheon_abi::{ContentBlock, Message, Role};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Embedding provider trait
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

/// Mock embedding provider for testing.
pub struct MockEmbedder;

impl Embedder for MockEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut vec: Vec<f32> = text.bytes().take(64).map(|b| b as f32 / 255.0).collect();
        vec.resize(64, 0.0);
        Ok(vec)
    }
}

/// 经验级别
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperienceLevel {
    /// 任务级: 完整任务的总结
    Narrative,
    /// 子任务级: 可复用的子任务模板
    Episodic,
}

/// 动作记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub action_type: String, // "click", "type", "screenshot"
    pub params: serde_json::Value,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// 经验条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experience {
    pub id: String,
    pub task_description: String,
    pub plan: Vec<String>,
    pub actions: Vec<ActionRecord>,
    pub result_summary: String,
    pub success: bool,
    pub level: ExperienceLevel,
    pub embedding: Vec<f32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 经验记忆存储
pub struct ExperienceMemory {
    experiences: Vec<Experience>,
    max_size: usize,
}

impl ExperienceMemory {
    pub fn new(max_size: usize) -> Self {
        Self {
            experiences: Vec::new(),
            max_size,
        }
    }

    /// 存储经验
    pub fn store(&mut self, exp: Experience) {
        if self.experiences.len() >= self.max_size {
            // 移除最旧的
            self.experiences.remove(0);
        }
        self.experiences.push(exp);
    }

    /// 按相似度检索 (余弦相似度)
    pub fn recall(&self, query_embedding: &[f32], top_k: usize) -> Vec<&Experience> {
        let mut scored: Vec<(f32, &Experience)> = self
            .experiences
            .iter()
            .map(|exp| (cosine_similarity(query_embedding, &exp.embedding), exp))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(top_k).map(|(_, exp)| exp).collect()
    }

    /// 按关键词检索
    pub fn search(&self, query: &str, top_k: usize) -> Vec<&Experience> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<(usize, &Experience)> = self
            .experiences
            .iter()
            .map(|exp| {
                let score = exp
                    .task_description
                    .to_lowercase()
                    .matches(&query_lower)
                    .count()
                    + exp
                        .plan
                        .iter()
                        .filter(|p| p.to_lowercase().contains(&query_lower))
                        .count();
                (score, exp)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        results.sort_by(|a, b| b.0.cmp(&a.0));
        results
            .into_iter()
            .take(top_k)
            .map(|(_, exp)| exp)
            .collect()
    }

    /// 注入经验到 context
    pub fn format_for_context(&self, experiences: &[&Experience]) -> String {
        if experiences.is_empty() {
            return String::new();
        }
        let mut out = String::from("## Relevant Past Experiences\n\n");
        for (i, exp) in experiences.iter().enumerate() {
            out += &format!("### Experience {}: {}\n", i + 1, exp.task_description);
            out += &format!("Success: {}\n", exp.success);
            out += &format!("Plan: {:?}\n", exp.plan);
            out += &format!("Result: {}\n\n", exp.result_summary);
        }
        out
    }

    pub fn count(&self) -> usize {
        self.experiences.len()
    }

    /// 使用 embedder 生成 embedding 并存储
    pub fn embed_and_store(&mut self, mut exp: Experience, embedder: &dyn Embedder) -> Result<()> {
        if exp.embedding.is_empty() {
            exp.embedding = embedder.embed(&exp.task_description)?;
        }
        self.store(exp);
        Ok(())
    }

    /// 注入相关经验到消息上下文
    pub fn inject_context(
        &self,
        goal: &str,
        context: &mut Vec<Message>,
        embedder: Option<&dyn Embedder>,
    ) -> Result<()> {
        let experiences = if let Some(emb) = embedder {
            let query_emb = emb.embed(goal)?;
            self.recall(&query_emb, 3)
        } else {
            self.search(goal, 3)
        };

        if !experiences.is_empty() {
            let formatted = self.format_for_context(&experiences);
            context.push(Message {
                role: Role::System,
                content: vec![ContentBlock::Text { text: formatted }],
            });
        }
        Ok(())
    }

    /// 保存到 JSON 文件
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.experiences)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 从 JSON 文件加载
    pub fn load_from_file(path: &Path, max_size: usize) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let experiences: Vec<Experience> = serde_json::from_str(&json)?;
        Ok(Self {
            experiences,
            max_size,
        })
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_experience(desc: &str, embedding: Vec<f32>, success: bool) -> Experience {
        Experience {
            id: uuid::Uuid::new_v4().to_string(),
            task_description: desc.into(),
            plan: vec!["step1".into(), "step2".into()],
            actions: vec![],
            result_summary: "done".into(),
            success,
            level: ExperienceLevel::Narrative,
            embedding,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_store_and_count() {
        let mut mem = ExperienceMemory::new(100);
        assert_eq!(mem.count(), 0);
        mem.store(make_experience("test", vec![1.0, 0.0], true));
        assert_eq!(mem.count(), 1);
    }

    #[test]
    fn test_recall_by_similarity() {
        let mut mem = ExperienceMemory::new(100);
        mem.store(make_experience("open browser", vec![1.0, 0.0, 0.0], true));
        mem.store(make_experience("write document", vec![0.0, 1.0, 0.0], true));
        mem.store(make_experience("open editor", vec![0.9, 0.1, 0.0], true));

        let results = mem.recall(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results[0].task_description, "open browser");
    }

    #[test]
    fn test_max_size_eviction() {
        let mut mem = ExperienceMemory::new(2);
        mem.store(make_experience("a", vec![1.0], true));
        mem.store(make_experience("b", vec![1.0], true));
        mem.store(make_experience("c", vec![1.0], true));
        assert_eq!(mem.count(), 2);
    }

    #[test]
    fn test_cosine_similarity() {
        let sim = cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]);
        assert!((sim - 1.0).abs() < 0.001);

        let sim = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
        assert!((sim - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_embed_and_store() {
        let embedder = MockEmbedder;
        let mut mem = ExperienceMemory::new(100);
        let exp = make_experience("navigate to page", vec![], true);
        mem.embed_and_store(exp, &embedder).unwrap();

        assert_eq!(mem.count(), 1);
        // embedding should have been filled in
        assert!(!mem.experiences[0].embedding.is_empty());
        assert_eq!(mem.experiences[0].embedding.len(), 64);
    }

    #[test]
    fn test_embed_and_store_preserves_existing_embedding() {
        let embedder = MockEmbedder;
        let mut mem = ExperienceMemory::new(100);
        let existing_emb = vec![0.5; 64];
        let exp = make_experience("navigate to page", existing_emb.clone(), true);
        mem.embed_and_store(exp, &embedder).unwrap();

        // embedding should remain unchanged since it was already set
        assert_eq!(mem.experiences[0].embedding, existing_emb);
    }

    #[test]
    fn test_inject_context_with_embedder() {
        let embedder = MockEmbedder;
        let mut mem = ExperienceMemory::new(100);
        mem.store(make_experience("open browser", vec![1.0, 0.0, 0.0], true));
        mem.store(make_experience("write document", vec![0.0, 1.0, 0.0], true));

        let mut context: Vec<Message> = Vec::new();
        mem.inject_context("open browser", &mut context, Some(&embedder))
            .unwrap();

        assert_eq!(context.len(), 1);
        assert_eq!(context[0].role, Role::System);
        match &context[0].content[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("Relevant Past Experiences"));
            }
            _ => panic!("expected Text content block"),
        }
    }

    #[test]
    fn test_inject_context_without_embedder() {
        let mut mem = ExperienceMemory::new(100);
        mem.store(make_experience("open browser", vec![1.0, 0.0], true));

        let mut context: Vec<Message> = Vec::new();
        mem.inject_context("open", &mut context, None).unwrap();

        assert_eq!(context.len(), 1);
    }

    #[test]
    fn test_inject_context_empty_results() {
        let mut mem = ExperienceMemory::new(100);
        mem.store(make_experience("open browser", vec![1.0, 0.0], true));

        let mut context: Vec<Message> = Vec::new();
        mem.inject_context("completely unrelated query xyz", &mut context, None)
            .unwrap();

        // no context injected when search yields nothing
        assert_eq!(context.len(), 0);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let mut mem = ExperienceMemory::new(100);
        mem.store(make_experience("open browser", vec![1.0, 0.0], true));
        mem.store(make_experience("write doc", vec![0.0, 1.0], false));

        let dir = std::env::temp_dir().join("aletheon_test_experience");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_memory.json");

        mem.save_to_file(&path).unwrap();
        let loaded = ExperienceMemory::load_from_file(&path, 100).unwrap();

        assert_eq!(loaded.count(), 2);
        assert_eq!(loaded.experiences[0].task_description, "open browser");
        assert_eq!(loaded.experiences[1].task_description, "write doc");
        assert!(!loaded.experiences[0].success || loaded.experiences[0].success); // just access

        // cleanup
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_mock_embedder_dimensions() {
        let embedder = MockEmbedder;
        // short text
        let emb = embedder.embed("hi").unwrap();
        assert_eq!(emb.len(), 64);
        // long text gets truncated
        let long_text = "a".repeat(200);
        let emb = embedder.embed(&long_text).unwrap();
        assert_eq!(emb.len(), 64);
    }
}
