use crate::{
    markdown::{Block, Kind, Style, Text},
    render::{Canvas, Color, Face, Glyphs, Graphics, Image, Rect, area},
};
use pulldown_cmark::Alignment;
use std::{
    borrow::Cow,
    collections::HashMap,
    ops::Range,
    path::{Component, Path, PathBuf, Prefix},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    run: usize,
    offset: u32,
}

pub struct Hit<'a> {
    pub cursor: Option<Cursor>,
    pub link: Option<&'a str>,
    pub text: bool,
}

#[derive(Clone, Copy)]
enum Source {
    Block(usize),
    Cell(usize, usize, usize),
    ImageAlt(usize),
}

struct Run {
    source: Source,
    rect: Rect,
    face: Face,
    align: Alignment,
    glyphs: Option<Glyphs>,
}

impl Run {
    fn layout(&mut self, gfx: &Graphics, text: &Text) -> Option<&mut Glyphs> {
        if self.glyphs.is_none() {
            self.glyphs = gfx
                .text(
                    text,
                    self.face,
                    self.align,
                    self.rect.right - self.rect.left,
                )
                .ok();
        }
        self.glyphs.as_mut()
    }
}

#[derive(Clone, Copy)]
enum Decoration {
    Body,
    Heading,
    Code { first: bool, last: bool },
    Rule,
    Table { header: bool },
    Image,
    ImageAlt,
}

struct Row {
    block: usize,
    rect: Rect,
    runs: Range<usize>,
    marker: bool,
    decoration: Decoration,
}

struct Picture {
    image: Option<Image>,
    label: Text,
    failed: bool,
}

pub struct View {
    pub path: PathBuf,
    blocks: Vec<Block>,
    image_unavailable: &'static str,
    pictures: HashMap<usize, Picture>,
    runs: Vec<Run>,
    rows: Vec<Row>,
    width: f32,
    height: f32,
    cached: Range<usize>,
}

impl View {
    pub fn new(path: PathBuf, blocks: Vec<Block>, image_unavailable: &'static str) -> Self {
        Self {
            path,
            blocks,
            image_unavailable,
            pictures: HashMap::new(),
            runs: Vec::new(),
            rows: Vec::new(),
            width: 0.0,
            height: 0.0,
            cached: 0..0,
        }
    }

    pub fn has_content(&self) -> bool {
        !self.blocks.is_empty()
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn reflow(&mut self, gfx: &Graphics, width: f32, scroll: f32) -> f32 {
        if (self.width - width).abs() < 0.1 {
            return scroll;
        }
        let top_row = self.row_at(scroll);
        let offset = self
            .rows
            .get(top_row)
            .map_or(0.0, |row| scroll - row.rect.top);
        self.width = width;
        self.runs.clear();
        self.rows.clear();

        let mut y = 39.0;
        for (index, block) in self.blocks.iter().enumerate() {
            let indent = ((block.quotes * 18 + block.list_depth * 28) as f32).min(width * 0.42);
            let column = (width - indent).max(1.0);
            let first = self.runs.len();
            let keep = y < scroll + 1350.0 && y > scroll - 500.0;
            let (rect, decoration, gap) = match &block.kind {
                Kind::Paragraph(text) => {
                    let rect = add_run(
                        &mut self.runs,
                        gfx,
                        text,
                        Source::Block(index),
                        area(indent, y, column, 26.0),
                        (Face::Body, Alignment::None),
                        keep,
                    );
                    let gap = if block.list_depth > 0 { 5.0 } else { 14.0 };
                    (rect, Decoration::Body, gap)
                }
                Kind::Heading { level, text, .. } => {
                    if index > 0 {
                        y += if *level <= 2 { 12.0 } else { 5.0 };
                    }
                    let rect = add_run(
                        &mut self.runs,
                        gfx,
                        text,
                        Source::Block(index),
                        area(indent, y, column, 26.0),
                        (Face::heading(*level), Alignment::None),
                        keep,
                    );
                    (
                        rect,
                        Decoration::Heading,
                        if *level == 1 { 15.0 } else { 9.0 },
                    )
                }
                Kind::Code { text, first, last } => {
                    let top = if *first { 12.0 } else { 0.0 };
                    let bottom = if *last { 12.0 } else { 0.0 };
                    let inner = add_run(
                        &mut self.runs,
                        gfx,
                        text,
                        Source::Block(index),
                        area(indent + 15.0, y + top, column - 30.0, 22.0),
                        (Face::Code, Alignment::None),
                        keep,
                    );
                    (
                        area(indent, y, column, inner.bottom - y + bottom),
                        Decoration::Code {
                            first: *first,
                            last: *last,
                        },
                        if *last { 15.0 } else { 0.0 },
                    )
                }
                Kind::Rule => (area(indent, y, column, 18.0), Decoration::Rule, 12.0),
                Kind::Table(table) => {
                    let widths = column_widths(table, column);
                    for (row_index, cells) in table.rows.iter().enumerate() {
                        let header = row_index == 0;
                        let face = if header { Face::TableHead } else { Face::Table };
                        let start = self.runs.len();
                        let mut x = indent;
                        let mut bottom = y + 41.0;
                        for (cell_index, (text, &cell_width)) in
                            cells.iter().zip(&widths).enumerate()
                        {
                            let pad = cell_padding(cell_width);
                            let rect = add_run(
                                &mut self.runs,
                                gfx,
                                text,
                                Source::Cell(index, row_index, cell_index),
                                area(x + pad, y + 9.0, cell_width - pad * 2.0, 23.0),
                                (face, table.alignments[cell_index]),
                                y < scroll + 1350.0 && y > scroll - 500.0,
                            );
                            bottom = bottom.max(rect.bottom + 9.0);
                            x += cell_width;
                        }
                        self.rows.push(Row {
                            block: index,
                            rect: area(indent, y, column, bottom - y),
                            runs: start..self.runs.len(),
                            marker: header && block.marker.is_some(),
                            decoration: Decoration::Table { header },
                        });
                        y = bottom;
                    }
                    y += 17.0;
                    continue;
                }
                Kind::Image { source, alt, .. } => {
                    let picture = self.pictures.entry(index).or_insert_with(|| Picture {
                        image: local_path(&self.path, source)
                            .and_then(|path| gfx.image(&path).ok()),
                        label: Text::new(if alt.is_empty() {
                            self.image_unavailable
                        } else {
                            alt
                        }),
                        failed: false,
                    });
                    if let Some(image) = &picture.image {
                        let scale = (column / image.width.max(1) as f32).min(1.0);
                        (
                            area(
                                indent,
                                y,
                                image.width.max(1) as f32 * scale,
                                image.height.max(1) as f32 * scale,
                            ),
                            Decoration::Image,
                            15.0,
                        )
                    } else {
                        let inner = add_run(
                            &mut self.runs,
                            gfx,
                            &picture.label,
                            Source::ImageAlt(index),
                            area(indent + 14.0, y + 10.0, column - 28.0, 21.0),
                            (Face::Small, Alignment::None),
                            keep,
                        );
                        (
                            area(indent, y, column, inner.bottom - y + 10.0),
                            Decoration::ImageAlt,
                            15.0,
                        )
                    }
                }
            };
            self.rows.push(Row {
                block: index,
                rect,
                runs: first..self.runs.len(),
                marker: block.marker.is_some(),
                decoration,
            });
            y = rect.bottom + gap;
        }
        self.height = y + 34.0;
        self.cached = self.row_at(scroll - 500.0)
            ..self
                .rows
                .partition_point(|row| row.rect.top < scroll + 1350.0);
        if scroll <= 0.0 {
            0.0
        } else {
            self.rows
                .get(top_row)
                .map_or(scroll, |row| row.rect.top + offset)
        }
    }

    pub fn paint(
        &mut self,
        gfx: &Graphics,
        canvas: &Canvas<'_>,
        top: f32,
        height: f32,
        scale: f32,
        selection: Option<(Cursor, Cursor)>,
    ) {
        let visible =
            self.row_at(top)..self.rows.partition_point(|row| row.rect.top < top + height);
        let keep = self.row_at(top - height * 0.6)
            ..self
                .rows
                .partition_point(|row| row.rect.top < top + height * 1.6);
        for index in self.cached.clone().filter(|index| !keep.contains(index)) {
            let row = &self.rows[index];
            for run in &mut self.runs[row.runs.clone()] {
                run.glyphs = None;
            }
            if let Some(image) = self
                .pictures
                .get_mut(&row.block)
                .and_then(|picture| picture.image.as_mut())
            {
                image.bitmap = None;
            }
        }
        self.cached = keep;
        let selected = selection.map(|(a, b)| (a.min(b), a.max(b)));

        for row in &self.rows[visible] {
            let block = &self.blocks[row.block];
            paint_frame(canvas, row, block.quotes);
            if row.marker
                && let Some(marker) = &block.marker
            {
                let marker_units = Text::new(marker);
                let width = (row.rect.left - 6.0).clamp(1.0, 78.0);
                if let Ok(glyphs) = gfx.text(&marker_units, Face::Body, Alignment::Right, width) {
                    canvas.text(
                        &glyphs,
                        row.rect.left - width - 6.0,
                        row.rect.top,
                        Color::Accent,
                        0..0,
                    );
                }
            }
            if matches!(row.decoration, Decoration::Image) {
                if let Some(picture) = self.pictures.get_mut(&row.block) {
                    if !picture.failed
                        && let Some(image) = &mut picture.image
                    {
                        picture.failed = image.paint(gfx, canvas, row.rect, scale).is_err();
                    }
                    if picture.failed {
                        canvas.round(row.rect, 7.0, Color::Surface);
                        if let Ok(glyphs) = gfx.text(
                            &picture.label,
                            Face::Small,
                            Alignment::Center,
                            row.rect.right - row.rect.left,
                        ) {
                            canvas.text(
                                &glyphs,
                                row.rect.left,
                                row.rect.top + 10.0,
                                Color::Muted,
                                0..0,
                            );
                        }
                    }
                }
                continue;
            }

            for index in row.runs.clone() {
                let run = &mut self.runs[index];
                let text = source_text(&self.blocks, &self.pictures, run.source);
                let rect = run.rect;
                let Some(glyphs) = run.layout(gfx, text) else {
                    continue;
                };
                let range = match selected {
                    Some((a, b)) if (a.run..=b.run).contains(&index) => {
                        let from = if index == a.run { a.offset } else { 0 };
                        let to = if index == b.run { b.offset } else { text.len() };
                        from..to
                    }
                    _ => 0..0,
                };
                let color = match row.decoration {
                    Decoration::Heading | Decoration::Table { header: true } => Color::Heading,
                    Decoration::ImageAlt => Color::Muted,
                    _ => Color::Body,
                };
                glyphs.decorate(canvas.surface, text);
                canvas.text(glyphs, rect.left, rect.top, color, range);
                if matches!(row.decoration, Decoration::Table { .. }) && index + 1 < row.runs.end {
                    let next = self.runs[index + 1].rect.left;
                    let x = (rect.right + next) * 0.5;
                    canvas.fill(
                        area(x, row.rect.top, 1.0, row.rect.bottom - row.rect.top),
                        Color::Edge,
                    );
                }
            }
        }
    }

    pub fn clear_device_resources(&mut self) {
        for run in &mut self.runs {
            run.glyphs = None;
        }
        for image in self
            .pictures
            .values_mut()
            .filter_map(|picture| picture.image.as_mut())
        {
            image.bitmap = None;
        }
    }

    pub fn hit(&mut self, gfx: &Graphics, x: f32, y: f32) -> Hit<'_> {
        let row_index = self.row_at(y).min(self.rows.len().saturating_sub(1));
        let Some(row) = self.rows.get(row_index) else {
            return Hit {
                cursor: None,
                link: None,
                text: false,
            };
        };
        let block = &self.blocks[row.block];
        let image_link = match &block.kind {
            Kind::Image {
                link: Some(link), ..
            } if contains(row.rect, x, y) => Some(link.as_str()),
            _ => None,
        };
        if row.runs.is_empty() {
            let cursor = if row.runs.start < self.runs.len() {
                Some(Cursor {
                    run: row.runs.start,
                    offset: 0,
                })
            } else {
                self.all().map(|(_, end)| end)
            };
            return Hit {
                cursor,
                link: image_link,
                text: false,
            };
        }
        let index = (row.runs.start..row.runs.end)
            .find(|&i| x < self.runs[i].rect.right)
            .unwrap_or(row.runs.end - 1);
        let run = &mut self.runs[index];
        let text = source_text(&self.blocks, &self.pictures, run.source);
        let rect = run.rect;
        let (offset, position, inside) = run.layout(gfx, text).map_or((0, 0, false), |glyphs| {
            glyphs.hit(x - rect.left, y - rect.top)
        });
        let link = if inside {
            text.spans.iter().find_map(|span| match &span.style {
                Style::Link(url) if span.range.contains(&position) => Some(url.as_str()),
                _ => None,
            })
        } else {
            None
        };
        Hit {
            cursor: Some(Cursor {
                run: index,
                offset: offset.min(text.len()),
            }),
            link: link.or(image_link),
            text: inside,
        }
    }

    pub fn all(&self) -> Option<(Cursor, Cursor)> {
        let index = self.runs.len().checked_sub(1)?;
        let text = source_text(&self.blocks, &self.pictures, self.runs[index].source);
        Some((
            Cursor { run: 0, offset: 0 },
            Cursor {
                run: index,
                offset: text.len(),
            },
        ))
    }

    pub fn copy(&self, (start, end): (Cursor, Cursor)) -> Vec<u16> {
        let (start, end) = (start.min(end), start.max(end));
        let mut output = Vec::new();
        if start == end {
            return output;
        }
        for index in start.run..=end.run {
            let run = &self.runs[index];
            let text = source_text(&self.blocks, &self.pictures, run.source);
            let from = if index == start.run { start.offset } else { 0 } as usize;
            let to = if index == end.run {
                end.offset
            } else {
                text.len()
            } as usize;
            if index != start.run {
                match (self.runs[index - 1].source, run.source) {
                    (Source::Cell(a, b, _), Source::Cell(c, d, _)) if a == c && b == d => {
                        output.push(9)
                    }
                    _ => output.extend([13, 10]),
                }
            }
            for &unit in &text.units[from..to] {
                if unit == 10 && output.last() != Some(&13) {
                    output.push(13);
                }
                output.push(unit);
            }
        }
        output
    }

    pub fn anchor(&self, name: &str) -> Option<f32> {
        if name.is_empty() {
            return Some(0.0);
        }
        self.rows
            .iter()
            .find_map(|row| match &self.blocks[row.block].kind {
                Kind::Heading { anchor, .. } if anchor == name => {
                    Some((row.rect.top - 22.0).max(0.0))
                }
                _ => None,
            })
    }

    fn row_at(&self, y: f32) -> usize {
        self.rows.partition_point(|row| row.rect.bottom <= y)
    }
}

fn cell_padding(width: f32) -> f32 {
    (width * 0.15).min(12.0)
}

fn column_widths(table: &crate::markdown::Table, width: f32) -> Vec<f32> {
    let count = table.alignments.len().max(1);
    let floor = (width / count as f32).min(65.0);
    let mut weights = vec![12.0f32; count];
    for row in table.rows.iter().take(24) {
        for (weight, cell) in weights.iter_mut().zip(row) {
            *weight = weight.max(cell.len().min(62) as f32 * 7.0);
        }
    }
    let remaining = (width - floor * count as f32).max(0.0);
    let total: f32 = weights.iter().sum();
    for weight in &mut weights {
        *weight = floor + remaining * *weight / total;
    }
    weights
}

fn add_run(
    runs: &mut Vec<Run>,
    gfx: &Graphics,
    text: &Text,
    source: Source,
    mut rect: Rect,
    (face, align): (Face, Alignment),
    keep: bool,
) -> Rect {
    let glyphs = gfx.text(text, face, align, rect.right - rect.left).ok();
    if let Some(glyphs) = &glyphs {
        rect.bottom = rect.top + glyphs.height;
    }
    runs.push(Run {
        source,
        rect,
        face,
        align,
        glyphs: if keep { glyphs } else { None },
    });
    rect
}

fn source_text<'a>(
    blocks: &'a [Block],
    pictures: &'a HashMap<usize, Picture>,
    source: Source,
) -> &'a Text {
    match source {
        Source::Block(index) => match &blocks[index].kind {
            Kind::Paragraph(text) | Kind::Heading { text, .. } | Kind::Code { text, .. } => text,
            _ => unreachable!("text run belongs to a text block"),
        },
        Source::Cell(index, row, cell) => match &blocks[index].kind {
            Kind::Table(table) => &table.rows[row][cell],
            _ => unreachable!("cell run belongs to a table"),
        },
        Source::ImageAlt(index) => &pictures[&index].label,
    }
}

fn paint_frame(canvas: &Canvas<'_>, row: &Row, quotes: usize) {
    let rect = row.rect;
    let height = rect.bottom - rect.top;
    let width = rect.right - rect.left;
    if quotes > 0 {
        let count = quotes.min(4);
        let bar = (rect.left - (count as f32 * 9.0 + 6.0)).max(0.0);
        for level in 0..count {
            canvas.round(
                area(bar + level as f32 * 9.0, rect.top - 2.0, 2.0, height + 4.0),
                1.0,
                Color::Quote,
            );
        }
    }
    match row.decoration {
        Decoration::Code { first, last } => {
            if first || last {
                canvas.round(rect, 7.0, Color::Surface);
            }
            if !(first && last) {
                let top = if first { 7.0 } else { 0.0 };
                let bottom = if last { 7.0 } else { 0.0 };
                canvas.fill(
                    area(rect.left, rect.top + top, width, height - top - bottom),
                    Color::Surface,
                );
            }
        }
        Decoration::ImageAlt => canvas.round(rect, 7.0, Color::Surface),
        Decoration::Rule => canvas.fill(area(rect.left, rect.top + 8.0, width, 1.0), Color::Edge),
        Decoration::Table { header } => {
            if header {
                canvas.fill(rect, Color::Surface);
                canvas.fill(area(rect.left, rect.top, width, 1.0), Color::Edge);
            }
            canvas.fill(area(rect.left, rect.bottom - 1.0, width, 1.0), Color::Edge);
            canvas.fill(area(rect.left, rect.top, 1.0, height), Color::Edge);
            canvas.fill(area(rect.right - 1.0, rect.top, 1.0, height), Color::Edge);
        }
        _ => (),
    }
}

pub fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub fn url_text(s: &str) -> Option<Cow<'_, str>> {
    if !s.contains('%') {
        return Some(Cow::Borrowed(s));
    }
    let mut result = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            result.push((hi * 16 + lo) as u8);
        } else {
            result.push(byte);
        }
    }
    String::from_utf8(result).ok().map(Cow::Owned)
}

pub fn local_path(document: &Path, source: &str) -> Option<PathBuf> {
    let path = source.split(['?', '#']).next()?;
    let path = url_text(path.strip_prefix("file:///").unwrap_or(path))?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let path = Path::new(path.as_ref());
    let absolute = match path.components().next()? {
        Component::Prefix(prefix)
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
                && path.has_root() =>
        {
            path.into()
        }
        Component::Prefix(_) | Component::RootDir => return None,
        _ => document.parent()?.join(path),
    };
    if absolute
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.to_string_lossy().contains(':')))
    {
        return None;
    }
    match absolute.components().next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            Some(absolute)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_encoded_local_links_without_resolving_network_paths() {
        let document = Path::new(r"C:\Docs\readme.md");
        assert_eq!(
            local_path(document, "images/%D0%90%20B.png#figure"),
            Some(PathBuf::from(r"C:\Docs\images/А B.png")),
        );
        assert_eq!(local_path(document, "https://example.com/image.png"), None);
        assert_eq!(local_path(document, "//server/folder/image.png"), None);
        assert_eq!(local_path(document, "image.png:stream"), None);
    }
}
