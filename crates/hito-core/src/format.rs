//! The message format: plain text with a small Markdown subset.
//!
//! Messages are stored exactly as typed. Clients parse them with [`parse`] and draw the result
//! themselves; nothing is ever interpreted as HTML, so a message cannot inject markup into any
//! client. Supported: fenced code blocks with a language (```` ```rust ````), inline `code`,
//! **bold**, *italic* or _italic_, ~~strike~~, `> quotes`, links (`http`, `https`, `mailto`
//! only) and `@mentions`.

/// A block of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Text lines; newlines inside are kept in [`Inline::Text`].
    Paragraph(Vec<Inline>),
    /// Lines that began with `> `.
    Quote(Vec<Inline>),
    /// A fenced code block. `lang` is whatever followed the opening fence, if anything.
    Code { lang: Option<String>, code: String },
}

/// A span inside a paragraph or quote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Strike(String),
    Code(String),
    /// A link whose scheme is `http`, `https` or `mailto`. Anything else stays text.
    Link(String),
    /// `@username`, without the `@`.
    Mention(String),
}

/// Parse a message body into blocks.
pub fn parse(body: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut lines = body.split('\n');

    fn flush(blocks: &mut Vec<Block>, buf: &mut Vec<&str>, quote: bool) {
        if buf.is_empty() {
            return;
        }
        let text = buf.join("\n");
        buf.clear();
        let inlines = inline(&text);
        blocks.push(if quote {
            Block::Quote(inlines)
        } else {
            Block::Paragraph(inlines)
        });
    }

    while let Some(line) = lines.next() {
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            flush(&mut blocks, &mut para, false);
            flush(&mut blocks, &mut quote, true);
            let lang = rest.trim();
            // A fence with text after the closing ``` on the same line is `inline code`, not a
            // block; only a bare word (or nothing) names a language.
            if lang.contains("```") || lang.contains(' ') {
                para.push(line);
                continue;
            }
            let lang = (!lang.is_empty()).then(|| lang.to_owned());
            let mut code: Vec<&str> = Vec::new();
            let mut closed = false;
            for l in lines.by_ref() {
                if l.trim() == "```" {
                    closed = true;
                    break;
                }
                code.push(l);
            }
            // An unclosed fence still shows as code: that is what the author meant.
            let _ = closed;
            blocks.push(Block::Code {
                lang,
                code: code.join("\n"),
            });
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("> ")
            .or_else(|| (line == ">").then_some(""))
        {
            flush(&mut blocks, &mut para, false);
            quote.push(rest);
            continue;
        }
        flush(&mut blocks, &mut quote, true);
        para.push(line);
    }
    flush(&mut blocks, &mut para, false);
    flush(&mut blocks, &mut quote, true);
    blocks
}

fn push_text(out: &mut Vec<Inline>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Inline::Text(t)) = out.last_mut() {
        t.push_str(s);
    } else {
        out.push(Inline::Text(s.to_owned()));
    }
}

/// Find the closing `marker` after `start`, on content that is not empty and does not begin or
/// end with a space (so `2 * 3 * 4` is not italic).
fn closing(s: &str, start: usize, marker: &str) -> Option<usize> {
    let rest = s.get(start..)?;
    let end = rest.find(marker)?;
    let inner = rest.get(..end)?;
    if inner.is_empty() || inner.starts_with(' ') || inner.ends_with(' ') || inner.contains('\n') {
        return None;
    }
    Some(start + end)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Parse the spans of a paragraph.
pub fn inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut plain_from = 0;
    while i < text.len() {
        let Some(rest) = text.get(i..) else { break };
        let prev = text.get(..i).and_then(|p| p.chars().next_back());
        let mut matched: Option<(Inline, usize)> = None;

        if rest.starts_with('`') {
            if let Some(end) = text.get(i + 1..).and_then(|r| r.find('`')) {
                let code = text.get(i + 1..i + 1 + end).unwrap_or_default();
                if !code.is_empty() {
                    matched = Some((Inline::Code(code.to_owned()), i + 1 + end + 1));
                }
            }
        } else if let Some(m) = ["**", "~~"].into_iter().find(|m| rest.starts_with(m)) {
            if let Some(end) = closing(text, i + 2, m) {
                let inner = text.get(i + 2..end).unwrap_or_default().to_owned();
                let span = if m == "**" {
                    Inline::Bold(inner)
                } else {
                    Inline::Strike(inner)
                };
                matched = Some((span, end + 2));
            }
        } else if (rest.starts_with('*') || rest.starts_with('_'))
            && !prev.is_some_and(is_word_char)
        {
            let m = if rest.starts_with('*') { "*" } else { "_" };
            if let Some(end) = closing(text, i + 1, m) {
                let after = text.get(end + 1..).and_then(|a| a.chars().next());
                if !after.is_some_and(is_word_char) {
                    let inner = text.get(i + 1..end).unwrap_or_default().to_owned();
                    matched = Some((Inline::Italic(inner), end + 1));
                }
            }
        } else if rest.starts_with('@') && !prev.is_some_and(is_word_char) {
            let name: String = rest
                .chars()
                .skip(1)
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .collect();
            let name = name.trim_end_matches(['.', '-']).to_owned();
            if name.len() >= crate::limits::USERNAME_MIN {
                let len = 1 + name.len();
                matched = Some((Inline::Mention(name.to_lowercase()), i + len));
            }
        } else if !prev.is_some_and(is_word_char)
            && ["https://", "http://", "mailto:"].iter().any(|s| {
                rest.len() > s.len()
                    && rest
                        .get(..s.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(s))
            })
        {
            let mut url: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | '`'))
                .collect();
            // Trailing punctuation usually ends the sentence, not the link.
            while url.ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '\'']) {
                url.pop();
            }
            let len = url.len();
            matched = Some((Inline::Link(url), i + len));
        }

        match matched {
            Some((span, next)) if next > i => {
                push_text(&mut out, text.get(plain_from..i).unwrap_or_default());
                out.push(span);
                i = next;
                plain_from = i;
            }
            _ => {
                i += rest.chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    push_text(&mut out, text.get(plain_from..).unwrap_or_default());
    out
}

/// Usernames mentioned in a body, without duplicates, in order of first appearance.
pub fn mentions(body: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for block in parse(body) {
        let spans = match block {
            Block::Paragraph(s) | Block::Quote(s) => s,
            Block::Code { .. } => continue,
        };
        for span in spans {
            if let Inline::Mention(name) = span {
                if !found.contains(&name) {
                    found.push(name);
                }
            }
        }
    }
    found
}

/// A one-line preview of a message for lists and notifications.
pub fn preview(body: &str, max_chars: usize) -> String {
    let flat: String = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("```", "");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut cut: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use Inline::*;

    #[test]
    fn plain_text_is_one_paragraph() {
        assert_eq!(
            parse("hola\nmundo"),
            vec![Block::Paragraph(vec![Text("hola\nmundo".into())])]
        );
    }

    #[test]
    fn fenced_code_keeps_indentation_and_language() {
        let body = "mira:\n```rust\nfn main() {\n    println!(\"*no* **bold**\");\n}\n```\nfin";
        assert_eq!(
            parse(body),
            vec![
                Block::Paragraph(vec![Text("mira:".into())]),
                Block::Code {
                    lang: Some("rust".into()),
                    code: "fn main() {\n    println!(\"*no* **bold**\");\n}".into()
                },
                Block::Paragraph(vec![Text("fin".into())]),
            ]
        );
    }

    #[test]
    fn unclosed_fence_is_still_code() {
        assert_eq!(
            parse("```\nlet x = 1;"),
            vec![Block::Code {
                lang: None,
                code: "let x = 1;".into()
            }]
        );
    }

    #[test]
    fn inline_spans() {
        assert_eq!(
            inline("**ojo** a `cargo test` y *esto*, ~~no~~ @Ana."),
            vec![
                Bold("ojo".into()),
                Text(" a ".into()),
                Code("cargo test".into()),
                Text(" y ".into()),
                Italic("esto".into()),
                Text(", ".into()),
                Strike("no".into()),
                Text(" ".into()),
                Mention("ana".into()),
                Text(".".into()),
            ]
        );
    }

    #[test]
    fn arithmetic_and_snake_case_are_not_italic() {
        assert_eq!(inline("2 * 3 * 4"), vec![Text("2 * 3 * 4".into())]);
        assert_eq!(
            inline("snake_case_name"),
            vec![Text("snake_case_name".into())]
        );
    }

    #[test]
    fn only_safe_link_schemes() {
        assert_eq!(
            inline("ver https://getnodalia.com/hito."),
            vec![
                Text("ver ".into()),
                Link("https://getnodalia.com/hito".into()),
                Text(".".into())
            ]
        );
        assert_eq!(
            inline("javascript:alert(1)"),
            vec![Text("javascript:alert(1)".into())]
        );
    }

    #[test]
    fn markup_is_never_interpreted() {
        assert_eq!(inline("<b>hi</b>"), vec![Text("<b>hi</b>".into())]);
    }

    #[test]
    fn quotes() {
        assert_eq!(
            parse("> citado\ndespués"),
            vec![
                Block::Quote(vec![Text("citado".into())]),
                Block::Paragraph(vec![Text("después".into())])
            ]
        );
    }

    #[test]
    fn emails_are_not_mentions_and_mentions_are_deduplicated() {
        assert_eq!(mentions("ana@example.com"), Vec::<String>::new());
        assert_eq!(
            mentions("@ana y @luis y @ana\n```\n@nadie\n```"),
            vec!["ana", "luis"]
        );
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        let s = "ñ*á*😀`ü`";
        assert_eq!(inline(s), vec![Text("ñ*á*😀".into()), Code("ü".into())]);
    }

    #[test]
    fn previews_are_one_line_and_bounded() {
        assert_eq!(preview("a\n\nb", 10), "a b");
        assert_eq!(preview("abcdefghij", 5), "abcd…");
    }
}
