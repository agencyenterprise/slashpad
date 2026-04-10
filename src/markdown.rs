//! Minimal markdown renderer. For the MVP we render the markdown as plain
//! text using pulldown-cmark to strip formatting into a flat string — iced's
//! text widget handles wrapping. A richer renderer that maps headings/code/
//! lists onto styled widgets is future work.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Flatten a markdown string into plain text with reasonable formatting.
pub fn render_plain(md: &str) -> String {
    let parser = Parser::new(md);
    let mut out = String::new();
    let mut in_code_block = false;
    for event in parser {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Start(Tag::Paragraph) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::Heading { .. }) => out.push('\n'),
            Event::End(TagEnd::Heading(_)) => out.push('\n'),
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                out.push('\n');
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push('\n');
            }
            Event::Start(Tag::Item) => out.push_str("• "),
            Event::End(TagEnd::Item) => out.push('\n'),
            _ => {}
        }
    }
    let _ = in_code_block;
    out.trim().to_string()
}
