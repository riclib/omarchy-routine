//! Four ways to say the same thing.
//!
//! Every human-facing command builds **markdown** and nothing else. `--pretty`
//! renders it for a terminal, `--md` prints it as written, `--txt` strips it
//! back to plain prose. That keeps one code path where there would otherwise be
//! three, and it composes: `rtn today --md` is valid input to `rtn log`.
//!
//! `--json` is the separate case, because a machine wants the fields rather
//! than a rendering of them.

use serde_json::Value;
use std::io::IsTerminal;
use termimad::crossterm::style::Color;
use termimad::MadSkin;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Pretty,
    Md,
    Txt,
    Json,
}

impl Format {
    /// Colour and layout when a person is watching; plain text when the output
    /// is going somewhere else. The same bargain `ls` and `git` make.
    pub fn default_for_stdout() -> Self {
        if std::io::stdout().is_terminal() {
            Format::Pretty
        } else {
            Format::Txt
        }
    }
}

/// A quieter skin than termimad's default, which shouts. Headers are the only
/// thing given real weight; everything else stays close to ordinary terminal
/// text so a day's log reads as prose rather than as a form.
fn skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.set_headers_fg(Color::AnsiValue(75));
    skin.bold.set_fg(Color::AnsiValue(255));
    skin.italic.set_fg(Color::AnsiValue(245));
    skin.inline_code.set_fg(Color::AnsiValue(210));
    skin.code_block.set_fg(Color::AnsiValue(252));
    skin.bullet.set_fg(Color::AnsiValue(75));
    skin
}

pub fn emit(markdown: &str, json: &Value, format: Format) {
    match format {
        Format::Json => println!("{json}"),
        Format::Md => println!("{}", markdown.trim_end()),
        Format::Pretty => skin().print_text(markdown),
        Format::Txt => println!("{}", to_plain(markdown).trim_end()),
    }
}

/// Markdown with its markers taken off. Not a renderer — the list markers and
/// the shape of the document stay, because they carry meaning here; only the
/// inline decoration goes.
pub fn to_plain(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let body = trimmed.trim_start_matches('#').trim_start_matches(' ');
        let keep = if trimmed.starts_with('#') { body } else { line };
        out.push_str(&strip_inline(keep));
        out.push('\n');
    }
    out
}

/// Drop `**`, `*`, `` ` `` and the bracket half of a link, keeping the text.
fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' | '`' => {
                i += 1;
            }
            // Underscore is emphasis only where it touches whitespace or an
            // edge. Inside a word it is part of the word -- an id, a path, a
            // column name -- and taking it out quietly corrupts the very thing
            // being reported back. `daily_notes__example` keeps all three.
            '_' if edge_underscore(&chars, i) => {
                i += 1;
            }
            '[' => {
                // [text](url) -> text. A bare [ with no closing pair is kept.
                match close_link(&chars, i) {
                    Some((text_end, after)) => {
                        out.extend(&chars[i + 1..text_end]);
                        i = after;
                    }
                    None => {
                        out.push('[');
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// True when an underscore sits against whitespace or the end of the line, the
/// only place markdown treats one as emphasis.
fn edge_underscore(chars: &[char], at: usize) -> bool {
    let open = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace());
    open(chars.get(at.wrapping_sub(1)).filter(|_| at > 0)) || open(chars.get(at + 1))
}

/// For a `[` at `open`, the index of its `]` and the index just past `(…)`.
fn close_link(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let close = (open + 1..chars.len()).find(|&i| chars[i] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|&i| chars[i] == ')')?;
    Some((close, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_decoration_comes_off() {
        assert_eq!(strip_inline("Spotify **plays** again"), "Spotify plays again");
        assert_eq!(strip_inline("press `SUPER+R` now"), "press SUPER+R now");
    }

    #[test]
    fn a_link_keeps_its_text_and_loses_its_url() {
        assert_eq!(
            strip_inline("see [chat](https://example.com/x) for it"),
            "see chat for it"
        );
    }

    #[test]
    fn an_underscore_inside_a_word_is_not_emphasis() {
        // An id is the thing most likely to be reported back, and the thing
        // most damaged by losing a character out of the middle of it.
        assert_eq!(strip_inline("task:aB0_cD1eF2gH3iJ4kL5m"), "task:aB0_cD1eF2gH3iJ4kL5m");
        assert_eq!(strip_inline("table:daily_notes__abc123"), "table:daily_notes__abc123");
        assert_eq!(strip_inline("see _that_ file"), "see that file");
    }

    #[test]
    fn an_unpaired_bracket_survives() {
        assert_eq!(strip_inline("a [ b ] c"), "a [ b ] c");
        assert_eq!(strip_inline("array[0] holds it"), "array[0] holds it");
    }

    #[test]
    fn headings_lose_their_hashes_and_keep_their_words() {
        assert_eq!(to_plain("## Notes\n- a thing"), "Notes\n- a thing\n");
    }

    #[test]
    fn a_fence_keeps_its_lines_and_loses_its_rails() {
        assert_eq!(
            to_plain("before\n```rust\nfn x() {}\n```\nafter"),
            "before\nfn x() {}\nafter\n"
        );
    }

    #[test]
    fn code_inside_a_fence_is_not_stripped() {
        // The asterisks here are a dereference, not emphasis.
        assert_eq!(to_plain("```c\nint y = *p;\n```"), "int y = *p;\n");
    }
}
