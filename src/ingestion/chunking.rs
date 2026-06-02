//! ChunkingService — splits document text into chunks (PRD FR-CHK-001/002/003).
//!
//! Markdown-aware: tracks heading paths and treats fenced code blocks as **atomic**
//! units that are never split across chunk boundaries (AC-5, Risk 4). Plain text
//! (.txt/.json/.yaml) is split by paragraph. Token counts use a word-count heuristic
//! (D-TOK) until a real tokenizer is introduced.

use crate::ingestion::domain::{chunk_id, content_hash, Chunk, ChunkingConfig};
use crate::ingestion::ports::ParsedDoc;
use std::collections::BTreeMap;

pub struct ChunkingService {
    config: ChunkingConfig,
}

/// An atomic unit of content that must not be split internally.
enum Unit {
    Text(String),
    Code(String),
}

struct Section {
    heading_path: Vec<String>,
    units: Vec<Unit>,
}

fn estimate_tokens(text: &str) -> u32 {
    text.split_whitespace().count() as u32
}

fn make_preview(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = flat.chars().take(120).collect();
    if flat.chars().count() > 120 {
        preview.push('…');
    }
    preview
}

/// Last `n` whitespace-delimited words of `text`, for inter-chunk overlap.
fn overlap_tail(text: &str, n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let start = words.len().saturating_sub(n as usize);
    words[start..].join(" ")
}

impl ChunkingService {
    pub fn new(config: ChunkingConfig) -> Self {
        Self { config }
    }

    pub fn chunk(&self, parsed: &ParsedDoc, ext: &str, document_id: &str) -> Vec<Chunk> {
        let sections = if ext.eq_ignore_ascii_case("md") {
            parse_markdown_sections(&parsed.text)
        } else {
            parse_plain_sections(&parsed.text)
        };

        let mut chunks = Vec::new();
        let mut index: u32 = 0;
        for section in sections {
            self.pack_section(&section, document_id, &mut index, &mut chunks);
        }
        chunks
    }

    fn pack_section(
        &self,
        section: &Section,
        document_id: &str,
        index: &mut u32,
        out: &mut Vec<Chunk>,
    ) {
        let target = self.config.target_tokens;
        let max = self.config.max_tokens;
        let overlap = self.config.overlap_tokens;

        let mut buf = String::new();
        let mut buf_tokens = 0u32;

        let flush = |buf: &mut String, buf_tokens: &mut u32, index: &mut u32, out: &mut Vec<Chunk>| {
            let content = buf.trim().to_string();
            if content.is_empty() {
                buf.clear();
                *buf_tokens = 0;
                return;
            }
            let ch = content_hash(content.as_bytes());
            out.push(Chunk {
                id: chunk_id(document_id, *index, &ch),
                document_id: document_id.to_string(),
                chunk_index: *index,
                heading_path: section.heading_path.clone(),
                preview: make_preview(&content),
                token_count: estimate_tokens(&content),
                content_hash: ch,
                content,
                metadata: BTreeMap::new(),
            });
            *index += 1;
            // seed next chunk with overlap tail for continuity
            let tail = overlap_tail(buf, overlap);
            *buf = tail.clone();
            *buf_tokens = estimate_tokens(&tail);
        };

        for unit in &section.units {
            match unit {
                Unit::Code(code) => {
                    let ct = estimate_tokens(code);
                    // Atomic: flush current buffer, then emit the code as its own chunk
                    // if it alone would exceed max (oversized, E2) or doesn't fit.
                    if !buf.trim().is_empty() && (buf_tokens + ct > max || ct > max) {
                        flush(&mut buf, &mut buf_tokens, index, out);
                    }
                    if ct > max {
                        // emit whole, oversized, no overlap carry
                        let content = code.trim().to_string();
                        let ch = content_hash(content.as_bytes());
                        out.push(Chunk {
                            id: chunk_id(document_id, *index, &ch),
                            document_id: document_id.to_string(),
                            chunk_index: *index,
                            heading_path: section.heading_path.clone(),
                            preview: make_preview(&content),
                            token_count: estimate_tokens(&content),
                            content_hash: ch,
                            content,
                            metadata: BTreeMap::new(),
                        });
                        *index += 1;
                        buf.clear();
                        buf_tokens = 0;
                        continue;
                    }
                    if !buf.is_empty() {
                        buf.push_str("\n\n");
                    }
                    buf.push_str(code);
                    buf_tokens += ct;
                }
                Unit::Text(text) => {
                    let tt = estimate_tokens(text);
                    if !buf.trim().is_empty() && buf_tokens + tt > max {
                        flush(&mut buf, &mut buf_tokens, index, out);
                    }
                    if !buf.is_empty() {
                        buf.push_str("\n\n");
                    }
                    buf.push_str(text);
                    buf_tokens += tt;
                }
            }
            if buf_tokens >= target {
                flush(&mut buf, &mut buf_tokens, index, out);
            }
        }
        flush(&mut buf, &mut buf_tokens, index, out);
    }
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn heading_level(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start();
    if !t.starts_with('#') {
        return None;
    }
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let title = t[hashes..].trim().to_string();
        Some((hashes, title))
    } else {
        None
    }
}

fn parse_markdown_sections(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut current_path: Vec<String> = Vec::new();

    let mut in_fence = false;
    let mut code_buf = String::new();
    let mut para_buf = String::new();

    let flush_para = |para_buf: &mut String, units: &mut Vec<Unit>| {
        let p = para_buf.trim();
        if !p.is_empty() {
            units.push(Unit::Text(p.to_string()));
        }
        para_buf.clear();
    };

    for line in text.lines() {
        if is_fence(line) {
            if in_fence {
                code_buf.push_str(line);
                units.push(Unit::Code(code_buf.trim_end().to_string()));
                code_buf.clear();
                in_fence = false;
            } else {
                flush_para(&mut para_buf, &mut units);
                code_buf.push_str(line);
                code_buf.push('\n');
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            code_buf.push_str(line);
            code_buf.push('\n');
            continue;
        }
        if let Some((level, title)) = heading_level(line) {
            flush_para(&mut para_buf, &mut units);
            if !units.is_empty() {
                sections.push(Section { heading_path: current_path.clone(), units: std::mem::take(&mut units) });
            }
            while heading_stack.last().map(|(l, _)| *l >= level).unwrap_or(false) {
                heading_stack.pop();
            }
            heading_stack.push((level, title));
            current_path = heading_stack.iter().map(|(_, t)| t.clone()).collect();
            continue;
        }
        if line.trim().is_empty() {
            flush_para(&mut para_buf, &mut units);
        } else {
            if !para_buf.is_empty() {
                para_buf.push('\n');
            }
            para_buf.push_str(line);
        }
    }
    // trailing
    if in_fence {
        units.push(Unit::Code(code_buf.trim_end().to_string()));
    }
    flush_para(&mut para_buf, &mut units);
    if !units.is_empty() {
        sections.push(Section { heading_path: current_path, units });
    }
    sections
}

fn parse_plain_sections(text: &str) -> Vec<Section> {
    let mut units = Vec::new();
    let mut para = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !para.trim().is_empty() {
                units.push(Unit::Text(para.trim().to_string()));
            }
            para.clear();
        } else {
            if !para.is_empty() {
                para.push('\n');
            }
            para.push_str(line);
        }
    }
    if !para.trim().is_empty() {
        units.push(Unit::Text(para.trim().to_string()));
    }
    if units.is_empty() {
        vec![]
    } else {
        vec![Section { heading_path: vec![], units }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(target: u32, max: u32, overlap: u32) -> ChunkingService {
        ChunkingService::new(ChunkingConfig { target_tokens: target, max_tokens: max, overlap_tokens: overlap })
    }

    fn parsed(text: &str) -> ParsedDoc {
        ParsedDoc { text: text.to_string(), title: None }
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        // edge case E1
        assert!(svc(5, 10, 0).chunk(&parsed("   \n\n  "), "md", "doc_1").is_empty());
    }

    #[test]
    fn preserves_heading_path() {
        let md = "# Top\n\nintro\n\n## Sub\n\nbody text here\n";
        let chunks = svc(50, 100, 0).chunk(&parsed(md), "md", "doc_1");
        assert!(chunks.iter().any(|c| c.heading_path == vec!["Top".to_string()]));
        assert!(chunks
            .iter()
            .any(|c| c.heading_path == vec!["Top".to_string(), "Sub".to_string()]));
    }

    #[test]
    fn never_splits_a_code_fence() {
        // AC-5: a large code block must stay intact within one chunk.
        let md = "# T\n\nintro words here\n\n```rust\none two three four five six seven eight nine ten\n```\n\nafter the code block\n";
        let chunks = svc(3, 5, 0).chunk(&parsed(md), "md", "doc_1");
        // Every chunk must have a balanced number of fence markers (0 or 2),
        // i.e. no chunk contains a half-open fence.
        for c in &chunks {
            let fences = c.content.matches("```").count();
            assert_eq!(fences % 2, 0, "chunk splits a code fence: {:?}", c.content);
        }
        // The full code body survives in exactly one chunk.
        let with_code: Vec<_> = chunks.iter().filter(|c| c.content.contains("```rust")).collect();
        assert_eq!(with_code.len(), 1);
        assert!(with_code[0].content.contains("ten"));
    }

    #[test]
    fn plain_text_splits_by_size_with_indices() {
        let text = "alpha beta gamma\n\ndelta epsilon zeta\n\neta theta iota";
        let chunks = svc(2, 3, 0).chunk(&parsed(text), "txt", "doc_1");
        assert!(chunks.len() >= 2);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index as usize, i);
            assert!(!c.preview.is_empty());
            assert!(c.token_count <= 3 || c.content.contains(' '));
        }
    }
}
