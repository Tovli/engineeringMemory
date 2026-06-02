//! File parsers (FileParser port). For M1, .txt/.json/.yaml/.yml are ingested as plain
//! text; structural extraction is future work. Non-UTF8 input errors (edge case E3).

use crate::ingestion::ports::{FileParser, ParsedDoc};

fn decode(raw: &[u8]) -> anyhow::Result<String> {
    std::str::from_utf8(raw)
        .map(|s| s.to_string())
        .map_err(|_| anyhow::anyhow!("file is not valid UTF-8"))
}

pub struct MarkdownParser;

impl FileParser for MarkdownParser {
    fn extensions(&self) -> &'static [&'static str] {
        &["md"]
    }
    fn parse(&self, raw: &[u8]) -> anyhow::Result<ParsedDoc> {
        let text = decode(raw)?;
        // Title = first level-1 heading, if any.
        let title = text.lines().find_map(|l| {
            l.trim_start()
                .strip_prefix("# ")
                .map(|rest| rest.trim().to_string())
        });
        Ok(ParsedDoc { text, title })
    }
}

pub struct PlainTextParser;

impl FileParser for PlainTextParser {
    fn extensions(&self) -> &'static [&'static str] {
        &["txt", "json", "yaml", "yml"]
    }
    fn parse(&self, raw: &[u8]) -> anyhow::Result<ParsedDoc> {
        Ok(ParsedDoc { text: decode(raw)?, title: None })
    }
}

/// Default parser set for M1.
pub fn default_parsers() -> Vec<Box<dyn FileParser>> {
    vec![Box::new(MarkdownParser), Box::new(PlainTextParser)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_extracts_title() {
        let p = MarkdownParser;
        let d = p.parse(b"# Hello World\n\nbody").unwrap();
        assert_eq!(d.title.as_deref(), Some("Hello World"));
    }

    #[test]
    fn non_utf8_errors() {
        // edge case E3
        assert!(PlainTextParser.parse(&[0xff, 0xfe, 0x00]).is_err());
    }
}
