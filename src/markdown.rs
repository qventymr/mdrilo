use crate::syntax::{self, Language, Token};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{collections::HashMap, fs, io, ops::Range, path::Path};

#[derive(Clone)]
pub enum Style {
    Strong,
    Emphasis,
    Strike,
    Mono,
    Link(String),
    Syntax(Token),
}

pub struct Span {
    pub range: Range<u32>,
    pub style: Style,
}

#[derive(Default)]
pub struct Text {
    pub units: Vec<u16>,
    pub spans: Vec<Span>,
}

impl Text {
    pub fn new(s: &str) -> Self {
        Self {
            units: s.encode_utf16().collect(),
            spans: Vec::new(),
        }
    }

    pub fn len(&self) -> u32 {
        self.units.len() as u32
    }

    pub fn push(&mut self, s: &str) {
        self.units.extend(s.encode_utf16());
    }

    pub fn style(&mut self, start: u32, style: Style) {
        if start < self.len() {
            self.spans.push(Span {
                range: start..self.len(),
                style,
            });
        }
    }
}

pub struct Table {
    pub alignments: Vec<Alignment>,
    pub rows: Vec<Vec<Text>>,
}

pub enum Kind {
    Paragraph(Text),
    Heading {
        level: usize,
        anchor: String,
        text: Text,
    },
    Code {
        text: Text,
        first: bool,
        last: bool,
    },
    Table(Table),
    Image {
        source: String,
        alt: String,
        link: Option<String>,
    },
    Rule,
}

pub struct Block {
    pub quotes: usize,
    pub list_depth: usize,
    pub marker: Option<String>,
    pub kind: Kind,
}

pub fn read(path: &Path) -> io::Result<Vec<Block>> {
    let bytes = fs::read(path)?;
    let source = if let Some(wide) = bytes.strip_prefix(&[0xff, 0xfe]) {
        decode_utf16(wide, u16::from_le_bytes)?
    } else if let Some(wide) = bytes.strip_prefix(&[0xfe, 0xff]) {
        decode_utf16(wide, u16::from_be_bytes)?
    } else {
        String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    };
    Ok(parse(source.trim_start_matches('\u{feff}')))
}

fn decode_utf16(bytes: &[u8], decode: fn([u8; 2]) -> u16) -> io::Result<String> {
    let (chunks, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    char::decode_utf16(chunks.iter().copied().map(decode))
        .collect::<Result<String, _>>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[derive(Default)]
struct Document {
    blocks: Vec<Block>,
    anchors: HashMap<String, usize>,
    text: Text,
    spans: Vec<(u32, Style)>,
    lists: Vec<Option<u64>>,
    items: Vec<Option<String>>,
    heading: Option<usize>,
    table: Option<Table>,
    code: Option<(Language, String)>,
    image: Option<(String, String, usize)>,
    quotes: usize,
}

pub fn parse(source: &str) -> Vec<Block> {
    let mut document = Document::default();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(source, options) {
        document.event(event);
    }
    document.flush();
    document.blocks
}

impl Document {
    fn block(&mut self, kind: Kind) {
        self.blocks.push(Block {
            quotes: self.quotes,
            list_depth: self.items.len(),
            marker: self.items.last_mut().and_then(Option::take),
            kind,
        });
    }

    fn take_text(&mut self) -> Text {
        let mut text = std::mem::take(&mut self.text);
        for (start, style) in &mut self.spans {
            text.style(*start, style.clone());
            *start = 0;
        }
        text
    }

    fn flush(&mut self) {
        if self.text.len() == 0 {
            return;
        }
        let text = self.take_text();
        let kind = match self.heading {
            Some(level) => Kind::Heading {
                level,
                anchor: self.anchor(&text),
                text,
            },
            None => Kind::Paragraph(text),
        };
        self.block(kind);
    }

    fn anchor(&mut self, text: &Text) -> String {
        let mut base = String::new();
        for ch in char::decode_utf16(text.units.iter().copied()).flatten() {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                base.extend(ch.to_lowercase());
            } else if ch.is_whitespace() {
                base.push('-');
            }
        }
        let Some(&next_suffix) = self.anchors.get(&base) else {
            self.anchors.insert(base.clone(), 1);
            return base;
        };
        let mut suffix = next_suffix;
        let anchor = loop {
            let candidate = format!("{base}-{suffix}");
            suffix += 1;
            if !self.anchors.contains_key(&candidate) {
                break candidate;
            }
        };
        self.anchors.insert(base, suffix);
        self.anchors.insert(anchor.clone(), 1);
        anchor
    }

    fn event(&mut self, event: Event<'_>) {
        if let Some((_, alt, depth)) = &mut self.image {
            match event {
                Event::Start(Tag::Image { .. }) => *depth += 1,
                Event::End(TagEnd::Image) if *depth == 0 => self.end_image(),
                Event::End(TagEnd::Image) => *depth -= 1,
                Event::Text(value) | Event::Code(value) => alt.push_str(&value),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                _ => (),
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(value) => match &mut self.code {
                Some((_, buffer)) => buffer.push_str(&value),
                None => self.text.push(&value),
            },
            Event::Code(value) | Event::InlineHtml(value) => {
                let start = self.text.len();
                self.text.push(&value);
                self.text.style(start, Style::Mono);
            }
            Event::Html(value) => self.text.push(value.trim_end_matches('\n')),
            Event::SoftBreak => self.text.push(" "),
            Event::HardBreak => self.text.push("\n"),
            Event::Rule => {
                self.flush();
                self.block(Kind::Rule);
            }
            Event::TaskListMarker(checked) => {
                if let Some(marker) = self.items.last_mut() {
                    *marker = Some(if checked { "☑\u{fe0e}" } else { "☐" }.into());
                }
            }
            _ => (),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        let style = match tag {
            Tag::Strong => Style::Strong,
            Tag::Emphasis => Style::Emphasis,
            Tag::Strikethrough => Style::Strike,
            Tag::Link { dest_url, .. } => Style::Link(dest_url.into_string()),
            _ => {
                self.start_block(tag);
                return;
            }
        };
        self.spans.push((self.text.len(), style));
    }

    fn start_block(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as usize);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quotes += 1;
            }
            Tag::HtmlBlock => self.flush(),
            Tag::CodeBlock(kind) => {
                self.flush();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => Language::from_info(&info),
                    CodeBlockKind::Indented => Language::Plain,
                };
                self.code = Some((language, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                if self.items.last().is_some_and(Option::is_some) {
                    self.block(Kind::Paragraph(Text::default()));
                }
                self.lists.push(start);
            }
            Tag::Item => {
                let marker = match self.lists.last_mut().and_then(Option::as_mut) {
                    Some(number) => {
                        let marker = format!("{number}.");
                        *number = number.saturating_add(1);
                        marker
                    }
                    None => "•".into(),
                };
                self.items.push(Some(marker));
            }
            Tag::Table(alignments) => {
                self.flush();
                self.table = Some(Table {
                    alignments,
                    rows: Vec::new(),
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(Vec::with_capacity(table.alignments.len()));
                }
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((dest_url.into_string(), String::new(), 0));
            }
            _ => (),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link => {
                if let Some((start, style)) = self.spans.pop() {
                    self.text.style(start, style);
                }
            }
            TagEnd::Paragraph | TagEnd::HtmlBlock => self.flush(),
            TagEnd::Heading(_) => {
                if self.text.len() == 0 {
                    let text = Text::default();
                    let anchor = self.anchor(&text);
                    self.block(Kind::Heading {
                        level: self.heading.unwrap_or(1),
                        anchor,
                        text,
                    });
                }
                self.flush();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes -= 1;
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                if self.items.last().is_some_and(Option::is_some) {
                    self.block(Kind::Paragraph(Text::default()));
                }
                self.items.pop();
            }
            TagEnd::CodeBlock => {
                if let Some((language, code)) = self.code.take() {
                    let code = code.strip_suffix('\n').unwrap_or(&code);
                    let chunks = code_chunks(syntax::highlight(code, language));
                    let count = chunks.len();
                    for (index, text) in chunks.into_iter().enumerate() {
                        self.block(Kind::Code {
                            text,
                            first: index == 0,
                            last: index + 1 == count,
                        });
                    }
                }
            }
            TagEnd::TableCell => {
                let text = self.take_text();
                if let Some(row) = self.table.as_mut().and_then(|table| table.rows.last_mut()) {
                    row.push(text);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.block(Kind::Table(table));
                }
            }
            _ => (),
        }
    }

    fn end_image(&mut self) {
        let Some((source, alt, _)) = self.image.take() else {
            return;
        };
        if self.table.is_some() || self.heading.is_some() {
            self.text.push(&alt);
            return;
        }
        self.flush();
        let link = self.spans.iter().rev().find_map(|(_, style)| match style {
            Style::Link(link) => Some(link.clone()),
            _ => None,
        });
        self.block(Kind::Image { source, alt, link });
    }
}

fn code_chunks(code: Text) -> Vec<Text> {
    // Keep shaping and highlighting a large fence proportional to the visible lines.
    const LINES: usize = 40;
    let mut ends = code
        .units
        .iter()
        .enumerate()
        .filter_map(|(offset, &unit)| (unit == u16::from(b'\n')).then_some(offset))
        .skip(LINES - 1)
        .step_by(LINES)
        .peekable();
    if ends.peek().is_none() {
        return vec![code];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let mut span_start = 0;
    for end in ends.chain([code.units.len()]) {
        let (from, to) = (start as u32, end as u32);
        while code
            .spans
            .get(span_start)
            .is_some_and(|span| span.range.end <= from)
        {
            span_start += 1;
        }
        let spans = code.spans[span_start..]
            .iter()
            .take_while(|span| span.range.start < to)
            .filter_map(|span| {
                let range = span.range.start.saturating_sub(from)..span.range.end.min(to) - from;
                if range.is_empty() {
                    None
                } else {
                    Some(Span {
                        range,
                        style: span.style.clone(),
                    })
                }
            })
            .collect();
        chunks.push(Text {
            units: code.units[start..end].to_vec(),
            spans,
        });
        start = end + 1;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_nested_list_context_and_unicode_anchors() {
        let blocks = parse("# Пример 🦀\n\n> 3. **Да**\n>    - [x] Пункт\n\n# Пример 🦀\n");
        assert_eq!(blocks.len(), 4);
        assert!(matches!(&blocks[0].kind, Kind::Heading { anchor, .. } if anchor == "пример-"));
        assert_eq!(blocks[1].quotes, 1);
        assert_eq!(blocks[1].marker.as_deref(), Some("3."));
        assert_eq!(blocks[2].list_depth, 2);
        assert_eq!(blocks[2].marker.as_deref(), Some("☑\u{fe0e}"));
        assert!(matches!(&blocks[3].kind, Kind::Heading { anchor, .. } if anchor == "пример--1"));
    }

    #[test]
    fn splits_long_code_without_losing_text_or_multiline_highlighting() {
        let code = format!("r#\"{}конец\"#;", "🦀 line\n".repeat(90));
        let blocks = parse(&format!("```rust,no_run\n{code}\n```\n"));
        assert_eq!(blocks.len(), 3);
        let mut chunks = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            let Kind::Code { text, first, last } = &block.kind else {
                panic!("expected code")
            };
            assert_eq!(*first, index == 0);
            assert_eq!(*last, index == blocks.len() - 1);
            assert!(matches!(text.spans[0].style, Style::Syntax(Token::String)));
            assert_eq!(text.spans[0].range.start, 0);
            chunks.push(String::from_utf16(&text.units).unwrap());
        }
        assert_eq!(chunks.join("\n"), code);
    }
}
