pub mod tool_search;

use aletheon_abi::tool::ToolExposure;

/// A single entry in the BM25 search catalog.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub description: String,
    pub tokens: Vec<String>,
    pub exposure: ToolExposure,
}

/// BM25 parameters and catalog for tool discovery.
///
/// Scoring formula per query term `q` in document `d`:
///
/// ```text
/// score(q, d) = IDF(q) * (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl / avg_dl))
/// IDF(q) = ln((N - n(q) + 0.5) / (n(q) + 0.5) + 1)
/// ```
///
/// where `tf` = term frequency in `d`, `n(q)` = number of docs containing `q`,
/// `dl` = document length, `avg_dl` = average document length, `N` = total docs.
#[derive(Debug)]
pub struct BM25Catalog {
    entries: Vec<CatalogEntry>,
    avg_dl: f64,
    k1: f64,
    b: f64,
}

impl BM25Catalog {
    /// Build a catalog from the given entries.
    pub fn build(entries: Vec<CatalogEntry>) -> Self {
        let avg_dl = if entries.is_empty() {
            0.0
        } else {
            entries.iter().map(|e| e.tokens.len() as f64).sum::<f64>() / entries.len() as f64
        };
        Self {
            entries,
            avg_dl,
            k1: 1.2,
            b: 0.75,
        }
    }

    /// Search the catalog with a query string.
    ///
    /// Returns `(name, score)` pairs sorted by descending score, filtered
    /// to only searchable exposures.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(&str, f64)> {
        let query_tokens = tokenize_and_stem(query);
        if query_tokens.is_empty() || self.entries.is_empty() {
            return Vec::new();
        }

        let n = self.entries.len() as f64;

        // Pre-compute IDF for each query term.
        let idf_map: std::collections::HashMap<String, f64> = query_tokens
            .iter()
            .map(|qt| {
                let doc_count = self
                    .entries
                    .iter()
                    .filter(|e| e.exposure.is_searchable() && e.tokens.contains(qt))
                    .count() as f64;
                let idf = ((n - doc_count + 0.5) / (doc_count + 0.5) + 1.0).ln();
                (qt.clone(), idf)
            })
            .collect();

        let mut scored: Vec<(&str, f64)> = self
            .entries
            .iter()
            .filter(|e| e.exposure.is_searchable())
            .map(|entry| {
                let dl = entry.tokens.len() as f64;
                let mut score = 0.0f64;

                for qt in &query_tokens {
                    let tf = entry.tokens.iter().filter(|t| *t == qt).count() as f64;
                    if tf == 0.0 {
                        continue;
                    }
                    let idf = idf_map[qt];
                    let numerator = tf * (self.k1 + 1.0);
                    let denominator = tf + self.k1 * (1.0 - self.b + self.b * dl / self.avg_dl);
                    score += idf * numerator / denominator;
                }

                (entry.name.as_str(), score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Clone the entries vector (used by `ToolSearchTool::boxed_clone`).
    pub fn entries_clone(&self) -> Vec<CatalogEntry> {
        self.entries.clone()
    }

    /// Get the description for a tool by name.
    pub fn get_description(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.description.as_str())
    }

    /// Number of entries in the catalog.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Tokenize and stem a text string into lowercase alphanumeric tokens.
///
/// Splits on non-alphanumeric boundaries and lowercases. A lightweight
/// stemmer strips common English suffixes (-ing, -tion, -ness, -ment,
/// -able, -ible, -ful, -less, -ly, -ed, -er, -es, -s).
pub fn tokenize_and_stem(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let lower = w.to_ascii_lowercase();
            stem(&lower)
        })
        .collect()
}

fn stem(word: &str) -> String {
    // Order matters: try longer suffixes first.
    const SUFFIXES: &[&str] = &[
        "ation", "tion", "ment", "ness", "able", "ible", "ful", "less", "ing", "ous", "ive",
        "ical", "ial", "ly", "ed", "er", "es", "al", "s",
    ];

    if word.len() <= 3 {
        return word.to_string();
    }

    for suffix in SUFFIXES {
        if word.ends_with(suffix) {
            let stem_len = word.len() - suffix.len();
            if stem_len >= 3 {
                return word[..stem_len].to_string();
            }
        }
    }
    word.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, desc: &str, exposure: ToolExposure) -> CatalogEntry {
        CatalogEntry {
            name: name.to_string(),
            description: desc.to_string(),
            tokens: tokenize_and_stem(desc),
            exposure,
        }
    }

    #[test]
    fn tokenize_basic() {
        let tokens = tokenize_and_stem("Execute a bash command");
        // "execute" has no matching suffix (no bare "e"), stays as "execute"
        assert_eq!(tokens, vec!["execute", "a", "bash", "command"]);
    }

    #[test]
    fn tokenize_strips_common_suffixes() {
        let tokens = tokenize_and_stem("processing execution management");
        // "processing" -> "process" (strips "ing"), "execution" -> "execu" (strips "tion"),
        // "management" -> "manage" (strips "ment")
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"execu".to_string()));
        assert!(tokens.contains(&"manage".to_string()));
    }

    #[test]
    fn catalog_build_and_len() {
        let entries = vec![
            make_entry("a", "tool alpha", ToolExposure::Direct),
            make_entry("b", "tool beta", ToolExposure::Deferred),
        ];
        let catalog = BM25Catalog::build(entries);
        assert_eq!(catalog.len(), 2);
        assert!(!catalog.is_empty());
    }

    #[test]
    fn search_returns_relevant_results() {
        let entries = vec![
            make_entry(
                "bash_exec",
                "Execute a bash command and return stdout",
                ToolExposure::Direct,
            ),
            make_entry(
                "file_read",
                "Read a file from the filesystem",
                ToolExposure::Direct,
            ),
            make_entry(
                "ebpf_compile",
                "Compile an eBPF program",
                ToolExposure::Deferred,
            ),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("bash command", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "bash_exec");
    }

    #[test]
    fn search_excludes_hidden_tools() {
        let entries = vec![
            make_entry("visible", "execute shell command", ToolExposure::Direct),
            make_entry("hidden_tool", "execute shell command", ToolExposure::Hidden),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("execute shell", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "visible");
    }

    #[test]
    fn search_includes_deferred_tools() {
        let entries = vec![
            make_entry("direct_tool", "compile kernel module", ToolExposure::Direct),
            make_entry(
                "deferred_tool",
                "compile kernel module",
                ToolExposure::Deferred,
            ),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("compile kernel", 10);
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"direct_tool"));
        assert!(names.contains(&"deferred_tool"));
    }

    #[test]
    fn search_excludes_direct_model_only() {
        let entries = vec![
            make_entry(
                "model_only",
                "special model tool",
                ToolExposure::DirectModelOnly,
            ),
            make_entry("regular", "special model tool", ToolExposure::Direct),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("special model", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "regular");
    }

    #[test]
    fn search_respects_limit() {
        let entries = vec![
            make_entry("a", "process data", ToolExposure::Direct),
            make_entry("b", "process data", ToolExposure::Direct),
            make_entry("c", "process data", ToolExposure::Direct),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("process", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn search_empty_query_returns_nothing() {
        let entries = vec![make_entry("a", "some tool", ToolExposure::Direct)];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn bm25_scores_longer_match_higher() {
        // A document with more matching terms should score higher.
        let entries = vec![
            make_entry("partial", "compile", ToolExposure::Direct),
            make_entry(
                "full_match",
                "compile kernel module build",
                ToolExposure::Direct,
            ),
        ];
        let catalog = BM25Catalog::build(entries);

        let results = catalog.search("compile kernel module", 10);
        assert_eq!(results.len(), 2);
        // full_match should score higher than partial
        assert_eq!(results[0].0, "full_match");
    }
}
