use async_trait::async_trait;
use xiaozhi_config::user::KnowledgeSearchHit;
use xiaozhi_core::Result;

use crate::traits::RagSearcher;

/// 本地关键词检索（文档由调用方注入）
pub struct LocalSearcher {
    documents: Vec<(String, String)>,
}

impl LocalSearcher {
    pub fn from_documents(docs: Vec<(String, String)>) -> Self {
        Self { documents: docs }
    }

    pub fn score(title: &str, content: &str, query: &str) -> f64 {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return 0.0;
        }
        let text = format!("{title} {content}").to_lowercase();
        if text.contains(&q) {
            return 1.0;
        }
        let words: Vec<&str> = q.split_whitespace().filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return 0.0;
        }
        let matched = words.iter().filter(|w| text.contains(*w)).count();
        matched as f64 / words.len() as f64
    }
}

#[async_trait]
impl RagSearcher for LocalSearcher {
    async fn search(
        &self,
        query: &str,
        _kb_id: &str,
        top_k: usize,
        threshold: f64,
    ) -> Result<Vec<KnowledgeSearchHit>> {
        let mut hits: Vec<KnowledgeSearchHit> = self
            .documents
            .iter()
            .filter_map(|(title, content)| {
                let score = Self::score(title, content, query);
                if score < threshold {
                    return None;
                }
                Some(KnowledgeSearchHit {
                    content: content.clone(),
                    title: title.clone(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(top_k);
        Ok(hits)
    }
}
