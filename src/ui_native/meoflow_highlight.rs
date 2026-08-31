//! MeoFlow 源码语法高亮（平台无关；Native / egui 共用）。

use crate::action::script_compiler::CompileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    Keyword,
    String,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

const KEYWORDS: &[&str] = &[
    "打开应用",
    "open_app",
    "打开链接",
    "open_url",
    "打开文件",
    "open_file",
    "执行命令",
    "shell",
    "激活应用",
    "activate",
    "播放宏",
    "play_macro",
    "playmacro",
    "快捷键",
    "hotkey",
    "等待",
    "wait",
    "输入",
    "type",
    "粘贴",
    "paste",
    "按键",
    "key",
    "点击",
    "click",
];

/// 返回源码中的高亮片段（字节索引，`end` 不含）。
pub fn highlight_meoflow(source: &str) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        highlight_line(content, line_start, &mut spans);
        line_start += line.len();
    }
    merge_spans(&mut spans);
    spans
}

pub fn line_count(source: &str) -> usize {
    if source.is_empty() {
        1
    } else {
        source.lines().count().max(1)
    }
}

pub fn gutter_text(lines: usize) -> String {
    (1..=lines.max(1))
        .map(|n| format!("{n:>3}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn error_line_numbers(err: &CompileError) -> Vec<usize> {
    err.diagnostics.iter().map(|d| d.line).collect()
}

/// UTF-16 长度（与 AppKit `NSRange` 一致）。
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// 字节区间 → UTF-16 `NSRange` 的 location/length。
pub fn byte_range_to_utf16(source: &str, start: usize, end: usize) -> (usize, usize) {
    let loc = utf16_len(&source[..start.min(source.len())]);
    let len = utf16_len(&source[start.min(source.len())..end.min(source.len())]);
    (loc, len)
}

fn highlight_line(line: &str, line_start: usize, out: &mut Vec<HighlightSpan>) {
    if line.is_empty() {
        return;
    }

    let mut i = 0usize;
    while i < line.len() {
        if line[i..].starts_with('#') {
            push_span(out, line_start + i, line_start + line.len(), TokenKind::Comment);
            return;
        }
        if line[i..].starts_with('"') {
            if let Some(end) = find_string_end(&line[i + 1..]) {
                let end_idx = i + 1 + end + 1;
                push_span(out, line_start + i, line_start + end_idx.min(line.len()), TokenKind::String);
                i = end_idx.min(line.len());
                continue;
            }
        }
        if line.as_bytes().get(i).is_some_and(|b| b.is_ascii_digit()) {
            let mut j = i + 1;
            while j < line.len() && (line.as_bytes()[j].is_ascii_digit() || line.as_bytes()[j] == b'.') {
                j += 1;
            }
            push_span(out, line_start + i, line_start + j, TokenKind::Number);
            i = j;
            continue;
        }
        if line[i..].starts_with(|c: char| c.is_whitespace()) {
            i += line[i..].chars().next().map_or(1, |c| c.len_utf8());
            continue;
        }
        if let Some(kw) = match_keyword(&line[i..]) {
            push_span(out, line_start + i, line_start + i + kw.len(), TokenKind::Keyword);
            i += kw.len();
            continue;
        }
        i += line[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
}

fn find_string_end(rest: &str) -> Option<usize> {
    let mut escaped = false;
    for (idx, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(idx);
        }
    }
    None
}

fn match_keyword(rest: &str) -> Option<&'static str> {
    KEYWORDS
        .iter()
        .copied()
        .filter(|kw| rest.starts_with(kw))
        .filter(|kw| {
            rest[kw.len()..]
                .chars()
                .next()
                .map_or(true, |c| c.is_whitespace())
        })
        .max_by_key(|kw| kw.len())
}

fn push_span(out: &mut Vec<HighlightSpan>, start: usize, end: usize, kind: TokenKind) {
    if start < end {
        out.push(HighlightSpan { start, end, kind });
    }
}

fn merge_spans(spans: &mut Vec<HighlightSpan>) {
    spans.sort_by_key(|s| (s.start, s.end));
    let mut merged: Vec<HighlightSpan> = Vec::with_capacity(spans.len());
    for span in spans.drain(..) {
        if let Some(last) = merged.last_mut() {
            if last.kind == span.kind && last.end >= span.start {
                last.end = last.end.max(span.end);
                continue;
            }
        }
        merged.push(span);
    }
    *spans = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_comment_and_keyword() {
        let src = "# 注释\n等待 100\n输入 \"hi\"";
        let spans = highlight_meoflow(src);
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment && s.start == 0));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword && src[s.start..].starts_with("等待")));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Number));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
    }

    #[test]
    fn gutter_text_padding() {
        assert_eq!(gutter_text(1), "  1");
        assert_eq!(gutter_text(12).lines().count(), 12);
    }

    #[test]
    fn utf16_ascii_matches_bytes() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(byte_range_to_utf16("等待 1", 0, 6).0, 0);
    }
}
