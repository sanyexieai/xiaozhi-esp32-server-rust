use xiaozhi_config::user::{KnowledgeBaseRef, KnowledgeSearchHit};

pub fn is_knowledge_base_available(kb: &KnowledgeBaseRef) -> bool {
    if kb.status.eq_ignore_ascii_case("inactive") {
        return false;
    }
    !kb.external_kb_id.trim().is_empty()
}

pub fn has_available_knowledge_bases(knowledge_bases: &[KnowledgeBaseRef]) -> bool {
    knowledge_bases.iter().any(is_knowledge_base_available)
}

pub fn collect_searchable_kb_ids(
    knowledge_bases: &[KnowledgeBaseRef],
    selected_ids: &[u64],
) -> Vec<u64> {
    knowledge_bases
        .iter()
        .filter(|kb| is_knowledge_base_available(kb))
        .filter(|kb| selected_ids.is_empty() || selected_ids.contains(&kb.id))
        .map(|kb| kb.id)
        .collect()
}

pub fn default_knowledge_search_threshold(
    knowledge_bases: &[KnowledgeBaseRef],
    kb_ids: &[u64],
) -> f64 {
    let selected: std::collections::HashSet<u64> = kb_ids.iter().copied().collect();
    for kb in knowledge_bases {
        if !kb_ids.is_empty() && !selected.contains(&kb.id) {
            continue;
        }
        if let Some(threshold) = kb.retrieval_threshold {
            if threshold > 0.0 {
                return threshold;
            }
        }
    }
    0.2
}

pub fn format_knowledge_hits_for_llm(hits: &[KnowledgeSearchHit]) -> String {
    let mut lines = Vec::new();
    for (idx, hit) in hits.iter().enumerate() {
        let mut content = hit.content.trim().to_string();
        if content.is_empty() {
            continue;
        }
        if content.chars().count() > 200 {
            content = content.chars().take(200).collect::<String>() + "...";
        }
        lines.push(format!("{}. {content}", idx + 1));
    }
    let msg = lines.join("\n");
    if msg.is_empty() {
        "已获取相关信息".to_string()
    } else {
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaozhi_config::user::{KnowledgeBaseRef, KnowledgeSearchHit};

    #[test]
    fn filters_inactive_knowledge_bases() {
        let kbs = vec![
            KnowledgeBaseRef {
                id: 1,
                status: "inactive".into(),
                external_kb_id: "kb1".into(),
                ..Default::default()
            },
            KnowledgeBaseRef {
                id: 2,
                external_kb_id: "kb2".into(),
                ..Default::default()
            },
        ];
        assert_eq!(collect_searchable_kb_ids(&kbs, &[]), vec![2]);
    }

    #[test]
    fn formats_hit_snippets() {
        let hits = vec![KnowledgeSearchHit {
            content: "hello world".into(),
            ..Default::default()
        }];
        assert_eq!(format_knowledge_hits_for_llm(&hits), "1. hello world");
    }
}
