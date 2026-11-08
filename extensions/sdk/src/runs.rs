//! Turns a comment's HTML into the styled runs the comments contract
//! carries, so readers never parse a site's markup.
//!
//! This reads the small part of HTML comment systems use: bold, italic,
//! struck and code text, links, line breaks, paragraphs, quotes and
//! spoilers. Everything else keeps its text and loses its tag, and scripts,
//! styles and images are dropped, since attached pictures go in a comment's
//! `images` instead.

use crate::{Run, RunStyle};

/// One run of unstyled text.
pub fn plain(text: impl Into<String>) -> Run {
    Run {
        text: text.into(),
        style: RunStyle::empty(),
        link: None,
    }
}

/// Reads `html` into runs, merging neighbours of the same style.
///
/// Spoilers are recognised by a `spoiler` class or `data-spoiler` attribute
/// on any element, and by `<details>`, as most comment systems mark them.
pub fn runs_from_html(html: &str) -> Vec<Run> {
    let mut reader = Reader {
        runs: Vec::new(),
        open: Vec::new(),
        skipping: 0,
    };
    let mut rest = html;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let Some(end) = rest.find('>') else {
                    reader.text(rest);
                    break;
                };
                reader.tag(&rest[1..end]);
                rest = &rest[end + 1..];
            }
            Some(start) => {
                reader.text(&rest[..start]);
                rest = &rest[start..];
            }
            None => {
                reader.text(rest);
                break;
            }
        }
    }
    reader.finish()
}

struct Open {
    name: String,
    style: RunStyle,
    link: Option<String>,
}

struct Reader {
    runs: Vec<Run>,
    open: Vec<Open>,
    /// Inside `<script>` or `<style>`, whose text is never shown.
    skipping: usize,
}

impl Reader {
    fn style(&self) -> (RunStyle, Option<String>) {
        let mut style = RunStyle::empty();
        let mut link = None;
        for open in &self.open {
            style |= open.style;
            if open.link.is_some() {
                link.clone_from(&open.link);
            }
        }
        (style, link)
    }

    fn text(&mut self, raw: &str) {
        if self.skipping > 0 {
            return;
        }
        // Source whitespace isn't layout: line breaks come from tags.
        let mut text = String::new();
        let mut space = false;
        for c in decode(raw).chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                space = true;
            } else {
                if space {
                    text.push(' ');
                }
                space = false;
                text.push(if c == '\u{a0}' { ' ' } else { c });
            }
        }
        if space {
            text.push(' ');
        }
        self.push(text);
    }

    fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let (style, link) = self.style();
        if let Some(last) = self.runs.last_mut()
            && last.style == style
            && last.link == link
        {
            last.text.push_str(&text);
            return;
        }
        self.runs.push(Run { text, style, link });
    }

    /// Ends the line: a `<br>` always does, up to one blank line, and a
    /// block's edge only when the text isn't already at a line's start.
    fn line_break(&mut self, hard: bool) {
        if self.skipping > 0 {
            return;
        }
        // Never start with a break.
        let Some(last) = self.runs.last_mut() else {
            return;
        };
        let trimmed = last.text.trim_end_matches(' ').len();
        last.text.truncate(trimmed);
        let at_start = last.text.ends_with('\n');
        if (hard && !last.text.ends_with("\n\n")) || !at_start {
            self.push("\n".to_owned());
        }
    }

    fn tag(&mut self, tag: &str) {
        if tag.starts_with('!') || tag.starts_with('?') {
            return;
        }
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim_end_matches('/');
        let name_end = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        let attributes = &body[name_end..];
        if closing {
            self.close(&name);
            return;
        }
        let block = matches!(
            name.as_str(),
            "p" | "div" | "blockquote" | "li" | "ul" | "ol" | "pre" | "details" | "summary"
        );
        match name.as_str() {
            "br" => return self.line_break(true),
            "script" | "style" => {
                if !tag.ends_with('/') {
                    self.skipping += 1;
                    self.open.push(Open {
                        name,
                        style: RunStyle::empty(),
                        link: None,
                    });
                }
                return;
            }
            "img" | "hr" | "input" | "meta" | "link" | "source" | "wbr" => return,
            _ => {}
        }
        if block {
            self.line_break(false);
        }
        let mut style = match name.as_str() {
            "b" | "strong" => RunStyle::BOLD,
            "i" | "em" | "cite" => RunStyle::ITALIC,
            "s" | "strike" | "del" => RunStyle::STRIKE,
            "code" | "kbd" | "samp" | "pre" => RunStyle::CODE,
            "blockquote" | "q" => RunStyle::QUOTE,
            "details" => RunStyle::SPOILER,
            _ => RunStyle::empty(),
        };
        let lowered = attributes.to_ascii_lowercase();
        if lowered.contains("data-spoiler")
            || attribute(attributes, "class")
                .is_some_and(|class| class.to_ascii_lowercase().contains("spoiler"))
        {
            style |= RunStyle::SPOILER;
        }
        let link = (name == "a")
            .then(|| attribute(attributes, "href"))
            .flatten()
            .map(|href| decode(&href))
            .filter(|href| href.starts_with("https://"));
        if !tag.ends_with('/') {
            self.open.push(Open { name, style, link });
        }
    }

    fn close(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|open| open.name == name) else {
            return;
        };
        for open in self.open.drain(at..) {
            if matches!(open.name.as_str(), "script" | "style") {
                self.skipping = self.skipping.saturating_sub(1);
            }
        }
        if matches!(
            name,
            "p" | "div" | "blockquote" | "li" | "ul" | "ol" | "pre" | "details" | "summary"
        ) {
            self.line_break(false);
        }
    }

    fn finish(mut self) -> Vec<Run> {
        if let Some(first) = self.runs.first_mut() {
            let trimmed = first.text.trim_start().to_owned();
            first.text = trimmed;
        }
        if let Some(last) = self.runs.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
        }
        self.runs.retain(|run| !run.text.is_empty());
        self.runs
    }
}

/// The value of attribute `name`, quoted or not.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let lowered = attributes.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lowered[from..].find(name) {
        let start = from + found;
        from = start + name.len();
        let before = lowered[..start].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let rest = attributes[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or_default(),
            _ => rest.split(char::is_whitespace).next().unwrap_or_default(),
        };
        return Some(value.to_owned());
    }
    None
}

/// Decodes the character references comment HTML uses.
fn decode(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        decoded.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            decoded.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let character = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32),
        };
        match character {
            Some(c) => {
                decoded.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(runs: &[Run]) -> Vec<(&str, RunStyle, Option<&str>)> {
        runs.iter()
            .map(|r| (r.text.as_str(), r.style, r.link.as_deref()))
            .collect()
    }

    #[test]
    fn styles_links_and_breaks() {
        let runs = runs_from_html(
            "<p>That ending was <b>great</b> &amp; <i>sad</i></p><p>See <a href=\"https://example.org/a?b=1&amp;c=2\">this</a><br>ok</p>",
        );
        assert_eq!(
            shown(&runs),
            [
                ("That ending was ", RunStyle::empty(), None),
                ("great", RunStyle::BOLD, None),
                (" & ", RunStyle::empty(), None),
                ("sad", RunStyle::ITALIC, None),
                ("\nSee ", RunStyle::empty(), None),
                (
                    "this",
                    RunStyle::empty(),
                    Some("https://example.org/a?b=1&c=2")
                ),
                ("\nok", RunStyle::empty(), None),
            ]
        );
    }

    #[test]
    fn spoilers_and_quotes() {
        let runs = runs_from_html(
            "<blockquote>who?</blockquote>He <span class=\"spoiler-text\">dies</span>.",
        );
        assert_eq!(
            shown(&runs),
            [
                ("who?", RunStyle::QUOTE, None),
                ("\nHe ", RunStyle::empty(), None),
                ("dies", RunStyle::SPOILER, None),
                (".", RunStyle::empty(), None),
            ]
        );
    }

    #[test]
    fn drops_scripts_images_and_unsafe_links() {
        let runs = runs_from_html(
            "<script>alert(1)</script><img src=x>Hi <a href=\"javascript:alert(1)\">there</a>",
        );
        assert_eq!(shown(&runs), [("Hi there", RunStyle::empty(), None)]);
    }
}
