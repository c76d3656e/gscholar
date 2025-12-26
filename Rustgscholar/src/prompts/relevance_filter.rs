//! Relevance filtering prompts for academic paper classification.
//!
//! Contains system and user prompt templates for LLM-based relevance filtering.

/// System prompt for academic paper relevance filtering
pub const SYSTEM_PROMPT: &str = r#"You are an academic literature relevance classifier. Your task is to determine if a paper is related to the target domain based ONLY on the provided fields (title/abstract/tldr/venue/journal/keywords).

Rules you MUST follow:
- Do NOT fabricate abstract or paper content.
- Evidence must come from input text; cite the exact keywords/phrases that triggered your judgment.
- Output "uncertain" when unsure; do not guess.
- Output MUST be valid JSON only (no extra text), for machine parsing.

Classification criteria:
- relevant: Title or abstract/tldr explicitly mentions core concepts, methods, data, or applications of the target domain.
- irrelevant: Text clearly belongs to another topic with no explainable connection to the target domain.
- uncertain: Insufficient information (e.g., no abstract/tldr) or only vague keyword matches without context support.

Important rules:
- Evidence priority: abstract_text > tldr > title > venue/journal.
- If abstract_text and tldr are both empty: use only title + venue/journal; output "uncertain" if unsure.
- Do not mark "relevant" just because a word looks similar; there must be contextual support.
- If both positive and negative signals exist, prefer "uncertain" and explain the conflict in reason.

Output format (strict JSON, no markdown):
{
  "label": "relevant" | "irrelevant" | "uncertain",
  "confidence": 0.0-1.0,
  "evidence": ["keyword1", "keyword2"],
  "reason": "Brief explanation in English"
}"#;

/// User prompt template for single paper filtering
/// Placeholders: {filter_keywords}, {paper_json}
pub const USER_PROMPT_TEMPLATE: &str = r#"Determine if the following paper is relevant to the target domain.

Target domain keywords/phrases:
{filter_keywords}

Paper data (JSON):
{paper_json}

Output strict JSON only (no markdown code blocks, no extra text):
{
  "label": "relevant" | "irrelevant" | "uncertain",
  "confidence": 0.0-1.0,
  "evidence": ["triggering keyword1", "keyword2", ...],
  "reason": "Brief explanation"
}"#;

/// Build user prompt with paper data
pub fn build_user_prompt(filter_keywords: &str, paper_json: &str) -> String {
    USER_PROMPT_TEMPLATE
        .replace("{filter_keywords}", filter_keywords)
        .replace("{paper_json}", paper_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_user_prompt() {
        let prompt = build_user_prompt("landslide, slope", r#"{"title": "test"}"#);
        assert!(prompt.contains("landslide, slope"));
        assert!(prompt.contains(r#"{"title": "test"}"#));
    }
}
