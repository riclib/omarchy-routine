//! Markdown to Routine blocks.
//!
//! Routine stores markdown as the source text of a block and renders it at
//! display time, so **inline** formatting needs no work here — bold, links,
//! backticks and mailto: all survive as written. Only block structure has to be
//! recognised, and getting one case right is the whole reason this exists: a
//! fenced code block is one `code` block, not thirty paragraphs.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Checkbox {
    /// `- [ ]` becomes a live task bound to the block. What capture wants.
    Task,
    /// `- [ ]` becomes an inert checkbox. What backfilling old days wants:
    /// weeks of finished checkboxes have no business arriving as open tasks,
    /// and a schedule cannot be taken off one again over MCP.
    Inert,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(String),
    Heading { content: String, level: u8 },
    Bullet { content: String, ordered: bool, depth: u8 },
    Check { content: String, checked: bool, depth: u8 },
    /// Carries no task id yet; one is minted before the write. Authoring a
    /// `todo` with a null task makes Routine and its Electron client each mint
    /// their own, so the id is never left for the server to fill in.
    Todo { content: String, checked: bool },
    Quote(String),
    Code { language: Option<String>, content: String },
    Divider,
}

impl Block {
    pub fn needs_task(&self) -> bool {
        matches!(self, Block::Todo { .. })
    }

    pub fn to_json(&self, task: Option<&str>) -> Value {
        match self {
            Block::Paragraph(c) => json!({ "type": "paragraph", "content": c }),
            Block::Heading { content, level } => {
                json!({ "type": "heading", "content": content, "level": level, "retracted": false })
            }
            Block::Bullet { content, ordered, depth } => json!({
                "type": "bullet",
                "list_type": if *ordered { "ordered" } else { "unordered" },
                "content": content,
                "depth": depth,
            }),
            Block::Check { content, checked, depth } => {
                json!({ "type": "check", "checked": checked, "content": content, "depth": depth })
            }
            Block::Todo { content, checked } => {
                json!({ "type": "todo", "checked": checked, "content": content, "task": task })
            }
            Block::Quote(c) => json!({ "type": "blockquote", "content": c }),
            Block::Code { language, content } => {
                json!({ "type": "code", "language": language, "content": content })
            }
            Block::Divider => json!({ "type": "divider" }),
        }
    }
}

/// A reference to a block already in the note, echoed back so that a whole-
/// document write behaves as an append. Nothing is re-serialised, so block
/// types this file cannot even model survive untouched.
pub fn existing(id: &str) -> Value {
    json!({ "type": "existing", "id": id })
}

fn fence(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    for marker in ["```", "~~~"] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some((marker, rest.trim()));
        }
    }
    None
}

fn depth_of(line: &str) -> u8 {
    let spaces = line.len() - line.trim_start().len();
    let tabs = line.chars().take_while(|c| *c == '\t').count();
    ((spaces / 2).max(tabs)).min(8) as u8
}

/// Strip a leading YAML frontmatter block, if the text opens with one.
pub fn strip_frontmatter(md: &str) -> &str {
    let rest = match md.strip_prefix("---\n") {
        Some(r) => r,
        None => return md,
    };
    match rest.find("\n---\n") {
        Some(end) => &rest[end + 5..],
        None => md,
    }
}

pub fn parse(md: &str, mode: Checkbox) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut lines = strip_frontmatter(md).lines().peekable();

    // Consecutive plain lines are one paragraph, as markdown means them to be.
    macro_rules! flush {
        () => {
            if !paragraph.is_empty() {
                blocks.push(Block::Paragraph(paragraph.join(" ")));
                paragraph.clear();
            }
        };
    }

    while let Some(line) = lines.next() {
        // A fence swallows everything up to its close, verbatim. This is the
        // case the whole parser exists for.
        if let Some((marker, language)) = fence(line) {
            flush!();
            let mut body = Vec::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with(marker) {
                    break;
                }
                body.push(inner);
            }
            blocks.push(Block::Code {
                language: (!language.is_empty()).then(|| language.to_owned()),
                content: body.join("\n"),
            });
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush!();
            continue;
        }

        if trimmed.len() >= 3
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '*')
                || trimmed.chars().all(|c| c == '_'))
        {
            flush!();
            blocks.push(Block::Divider);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('#') {
            let extra = rest.chars().take_while(|c| *c == '#').count();
            let level = (1 + extra).min(6) as u8;
            let text = trimmed[level as usize..].trim_start();
            if !text.is_empty() {
                flush!();
                blocks.push(Block::Heading { content: text.to_owned(), level });
                continue;
            }
        }

        if let Some(rest) = trimmed.strip_prefix("> ") {
            flush!();
            blocks.push(Block::Quote(rest.to_owned()));
            continue;
        }

        // A checkbox is a list item first, so it has to be tested before one.
        let bullet = ["- ", "* ", "+ "].iter().find_map(|m| trimmed.strip_prefix(m));
        let ordered = trimmed
            .find(". ")
            .filter(|i| *i > 0 && trimmed[..*i].chars().all(|c| c.is_ascii_digit()))
            .map(|i| &trimmed[i + 2..]);

        if let Some(item) = bullet {
            flush!();
            let depth = depth_of(line);
            let checked = match item.get(..4) {
                Some("[ ] ") => Some(false),
                Some("[x] ") | Some("[X] ") => Some(true),
                _ => None,
            };
            match (checked, mode) {
                (Some(done), Checkbox::Task) => {
                    blocks.push(Block::Todo { content: item[4..].to_owned(), checked: done })
                }
                (Some(done), Checkbox::Inert) => blocks.push(Block::Check {
                    content: item[4..].to_owned(),
                    checked: done,
                    depth,
                }),
                (None, _) => blocks.push(Block::Bullet {
                    content: item.to_owned(),
                    ordered: false,
                    depth,
                }),
            }
            continue;
        }

        if let Some(item) = ordered {
            flush!();
            blocks.push(Block::Bullet {
                content: item.to_owned(),
                ordered: true,
                depth: depth_of(line),
            });
            continue;
        }

        paragraph.push(trimmed.to_owned());
    }
    flush!();
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_task(md: &str) -> Vec<Block> {
        parse(md, Checkbox::Task)
    }

    #[test]
    fn a_fence_is_one_block_however_long() {
        let md = "before\n\n```rust\nfn main() {\n    // - not a bullet\n    # not a heading\n}\n```\n\nafter";
        let b = parse_task(md);
        assert_eq!(b.len(), 3, "{b:#?}");
        assert_eq!(b[0], Block::Paragraph("before".into()));
        assert_eq!(
            b[1],
            Block::Code {
                language: Some("rust".into()),
                content: "fn main() {\n    // - not a bullet\n    # not a heading\n}".into(),
            }
        );
        assert_eq!(b[2], Block::Paragraph("after".into()));
    }

    #[test]
    fn a_fence_with_no_language_still_holds_together() {
        let b = parse_task("```\nplain\nlines\n```");
        assert_eq!(b, vec![Block::Code { language: None, content: "plain\nlines".into() }]);
    }

    #[test]
    fn checkboxes_follow_the_mode() {
        let md = "- [ ] open\n- [x] done";
        assert_eq!(
            parse(md, Checkbox::Task),
            vec![
                Block::Todo { content: "open".into(), checked: false },
                Block::Todo { content: "done".into(), checked: true },
            ]
        );
        assert_eq!(
            parse(md, Checkbox::Inert),
            vec![
                Block::Check { content: "open".into(), checked: false, depth: 0 },
                Block::Check { content: "done".into(), checked: true, depth: 0 },
            ]
        );
    }

    #[test]
    fn a_todo_never_authors_a_null_task() {
        let b = Block::Todo { content: "x".into(), checked: false };
        assert!(b.needs_task());
        assert_eq!(b.to_json(Some("task:abc"))["task"], json!("task:abc"));
    }

    #[test]
    fn plain_lines_join_into_one_paragraph() {
        let b = parse_task("one\ntwo\n\nthree");
        assert_eq!(
            b,
            vec![Block::Paragraph("one two".into()), Block::Paragraph("three".into())]
        );
    }

    #[test]
    fn headings_bullets_quotes_and_rules() {
        let b = parse_task("## Notes\n- a\n  - nested\n1. first\n> quoted\n---");
        assert_eq!(b[0], Block::Heading { content: "Notes".into(), level: 2 });
        assert_eq!(b[1], Block::Bullet { content: "a".into(), ordered: false, depth: 0 });
        assert_eq!(b[2], Block::Bullet { content: "nested".into(), ordered: false, depth: 1 });
        assert_eq!(b[3], Block::Bullet { content: "first".into(), ordered: true, depth: 0 });
        assert_eq!(b[4], Block::Quote("quoted".into()));
        assert_eq!(b[5], Block::Divider);
    }

    #[test]
    fn frontmatter_goes_away() {
        let md = "---\ntype: 'DailyNote'\ntitle: September 2, 2026\n---\n\nreal content";
        assert_eq!(parse_task(md), vec![Block::Paragraph("real content".into())]);
    }

    #[test]
    fn a_bare_hash_is_not_a_heading() {
        assert_eq!(parse_task("#"), vec![Block::Paragraph("#".into())]);
    }
}
