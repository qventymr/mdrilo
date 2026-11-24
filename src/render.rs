use crate::{
    markdown::{Style, Text},
    syntax::Token,
};
use pulldown_cmark::Alignment;
use std::{cell::OnceCell, ops::Range, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{GENERIC_READ, RECT},
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::HDC,
            Imaging::*,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::{BOOL, Interface, PCWSTR, Result, w},
};
use windows_numerics::Vector2;

#[derive(Clone, Copy)]
pub enum Color {
    Page,
    Body,
    Heading,
    Muted,
    Accent,
    AccentSurface,
    Surface,
    Edge,
    Quote,
    Selection,
    Thumb,
    Keyword,
    Literal,
    Number,
    Comment,
    Name,
}

const COLORS: [u32; 16] = [
    0x17191f, 0xd2d7e3, 0xf1f3fa, 0x98a1b3, 0xb4bfff, 0x2a3048, 0x20242e, 0x343a48, 0x7d8aca,
    0x3b4874, 0x4a5265, 0xc9aafb, 0xabd7a3, 0xf0c092, 0x919caf, 0x93c9ef,
];

fn rgba(value: u32) -> D2D1_COLOR_F {
    let channel = |shift| ((value >> shift) & 255u32) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

pub type Rect = D2D_RECT_F;

pub fn area(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

#[derive(Clone, Copy)]
pub enum Face {
    Body,
    H1,
    H2,
    H3,
    H4,
    Small,
    Label,
    Code,
    Table,
    TableHead,
}

impl Face {
    pub fn heading(level: usize) -> Self {
        match level {
            1 => Self::H1,
            2 => Self::H2,
            3 => Self::H3,
            4 => Self::H4,
            _ => Self::Label,
        }
    }

    fn metrics(self) -> (f32, f32, f32, DWRITE_FONT_WEIGHT) {
        let regular = DWRITE_FONT_WEIGHT_NORMAL;
        let medium = DWRITE_FONT_WEIGHT_SEMI_BOLD;
        match self {
            Self::Body => (17.0, 26.0, 19.0, regular),
            Self::H1 => (36.0, 48.0, 37.0, medium),
            Self::H2 => (27.0, 36.0, 27.0, medium),
            Self::H3 => (21.0, 29.0, 22.0, medium),
            Self::H4 => (18.0, 27.0, 20.0, medium),
            Self::Small => (14.0, 21.0, 15.5, regular),
            Self::Label => (16.0, 24.0, 18.0, medium),
            Self::Code => (15.0, 22.0, 16.0, regular),
            Self::Table => (15.5, 23.0, 17.0, regular),
            Self::TableHead => (15.5, 23.0, 17.0, medium),
        }
    }
}

pub struct Graphics {
    d2d: ID2D1Factory,
    write: IDWriteFactory,
    formats: [OnceCell<Result<IDWriteTextFormat>>; 10],
    imaging: OnceCell<Result<IWICImagingFactory>>,
}

impl Graphics {
    pub fn new() -> Result<Self> {
        unsafe {
            Ok(Self {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                formats: Default::default(),
                imaging: OnceCell::new(),
            })
        }
    }

    pub fn surface(&self) -> Result<Surface> {
        let options = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            ..Default::default()
        };
        unsafe {
            let target = self.d2d.CreateDCRenderTarget(&options)?;
            let mut brushes = Vec::with_capacity(COLORS.len());
            for value in COLORS {
                brushes.push(target.CreateSolidColorBrush(&rgba(value), None)?);
            }
            Ok(Surface { target, brushes })
        }
    }

    fn format(&self, face: Face) -> Result<&IDWriteTextFormat> {
        self.formats[face as usize]
            .get_or_init(|| unsafe {
                let (size, line, baseline, weight) = face.metrics();
                let font = if matches!(face, Face::Code) {
                    w!("Consolas")
                } else {
                    w!("Segoe UI")
                };
                let format = self.write.CreateTextFormat(
                    font,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )?;
                format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, baseline)?;
                if matches!(face, Face::Code) {
                    format.SetIncrementalTabStop(33.0)?;
                }
                Ok(format)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn text(
        &self,
        text: &Text,
        face: Face,
        alignment: Alignment,
        width: f32,
    ) -> Result<Glyphs> {
        unsafe {
            let font = self.format(face)?;
            let layout =
                self.write
                    .CreateTextLayout(&text.units, font, width.max(1.0), f32::MAX)?;
            let alignment = match alignment {
                Alignment::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Alignment::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
                _ => DWRITE_TEXT_ALIGNMENT_LEADING,
            };
            layout.SetTextAlignment(alignment)?;
            for span in &text.spans {
                let range = native_range(&span.range);
                match span.style {
                    Style::Strong => layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?,
                    Style::Emphasis => layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?,
                    Style::Strike => layout.SetStrikethrough(true, range)?,
                    Style::Link(_) => layout.SetUnderline(true, range)?,
                    Style::Mono => {
                        layout.SetFontFamilyName(w!("Consolas"), range)?;
                        layout.SetFontSize(15.5, range)?;
                    }
                    Style::Syntax(_) => (),
                }
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok(Glyphs {
                layout,
                height: metrics.height.max(face.metrics().1),
                decorated: false,
                chips: Vec::new(),
            })
        }
    }

    pub fn image(&self, path: &Path) -> Result<Image> {
        unsafe {
            let factory = self
                .imaging
                .get_or_init(|| {
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                })
                .as_ref()
                .map_err(Clone::clone)?;
            let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            let decoder = factory.CreateDecoderFromFilename(
                PCWSTR(name.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = decoder.GetFrame(0)?;
            let (mut width, mut height) = (0, 0);
            frame.GetSize(&mut width, &mut height)?;
            Ok(Image {
                source: frame.cast()?,
                width,
                height,
                bitmap: None,
            })
        }
    }
}

pub struct Image {
    source: IWICBitmapSource,
    pub width: u32,
    pub height: u32,
    pub bitmap: Option<(u32, u32, ID2D1Bitmap)>,
}

impl Image {
    pub fn paint(
        &mut self,
        gfx: &Graphics,
        canvas: &Canvas<'_>,
        rect: Rect,
        scale: f32,
    ) -> Result<()> {
        let width = (rect.right - rect.left) * scale;
        let height = (rect.bottom - rect.top) * scale;
        let limit = (4096.0 / width.max(height).max(1.0)).min(1.0);
        let width = (width * limit).ceil() as u32;
        let height = (height * limit).ceil() as u32;
        let width = width.clamp(1, self.width.max(1));
        let height = height.clamp(1, self.height.max(1));
        if !self
            .bitmap
            .as_ref()
            .is_some_and(|(w, h, _)| *w == width && *h == height)
        {
            let Some(Ok(factory)) = gfx.imaging.get() else {
                return Ok(());
            };
            unsafe {
                let source = if width != self.width || height != self.height {
                    let scaler = factory.CreateBitmapScaler()?;
                    scaler.Initialize(
                        &self.source,
                        width,
                        height,
                        WICBitmapInterpolationModeFant,
                    )?;
                    scaler.cast::<IWICBitmapSource>()?
                } else {
                    self.source.clone()
                };
                let converter = factory.CreateFormatConverter()?;
                converter.Initialize(
                    &source,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )?;
                let bitmap = canvas
                    .surface
                    .target
                    .CreateBitmapFromWicBitmap(&converter, None)?;
                self.bitmap = Some((width, height, bitmap));
            }
        }
        if let Some((_, _, bitmap)) = &self.bitmap {
            let rect = canvas.offset(rect);
            unsafe {
                canvas.surface.target.DrawBitmap(
                    bitmap,
                    Some(&rect),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                )
            };
        }
        Ok(())
    }
}

fn native_range(range: &Range<u32>) -> DWRITE_TEXT_RANGE {
    DWRITE_TEXT_RANGE {
        startPosition: range.start,
        length: range.end - range.start,
    }
}

pub struct Glyphs {
    pub height: f32,
    layout: IDWriteTextLayout,
    decorated: bool,
    chips: Vec<Rect>,
}

impl Glyphs {
    pub fn hit(&self, x: f32, y: f32) -> (u32, u32, bool) {
        let (mut trailing, mut inside, mut hit) =
            (BOOL(0), BOOL(0), DWRITE_HIT_TEST_METRICS::default());
        unsafe {
            let _ = self
                .layout
                .HitTestPoint(x, y, &mut trailing, &mut inside, &mut hit);
        }
        let cursor = hit.textPosition + if trailing.as_bool() { hit.length } else { 0 };
        (cursor, hit.textPosition, inside.as_bool())
    }

    pub fn ranges(&self, range: Range<u32>, mut draw: impl FnMut(Rect)) {
        if range.is_empty() {
            return;
        }
        let mut stack = [DWRITE_HIT_TEST_METRICS::default(); 8];
        let mut count = 0;
        let native = native_range(&range);
        let result = unsafe {
            self.layout.HitTestTextRange(
                native.startPosition,
                native.length,
                0.0,
                0.0,
                Some(&mut stack),
                &mut count,
            )
        };
        let mut heap;
        let boxes = if result.is_ok() {
            &stack[..count as usize]
        } else if count as usize > stack.len() {
            heap = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
            if unsafe {
                self.layout.HitTestTextRange(
                    native.startPosition,
                    native.length,
                    0.0,
                    0.0,
                    Some(&mut heap),
                    &mut count,
                )
            }
            .is_err()
            {
                return;
            }
            &heap[..count as usize]
        } else {
            return;
        };
        for hit in boxes {
            if hit.width > 0.0 {
                draw(area(hit.left, hit.top, hit.width, hit.height));
            }
        }
    }

    pub fn decorate(&mut self, surface: &Surface, text: &Text) {
        if self.decorated {
            return;
        }
        let mut chips = Vec::new();
        for span in &text.spans {
            let color = match span.style {
                Style::Link(_) => Color::Accent,
                Style::Mono => {
                    self.ranges(span.range.clone(), |rect| chips.push(rect));
                    continue;
                }
                Style::Syntax(token) => match token {
                    Token::Keyword => Color::Keyword,
                    Token::String => Color::Literal,
                    Token::Number => Color::Number,
                    Token::Comment => Color::Comment,
                    Token::Name => Color::Name,
                },
                _ => continue,
            };
            unsafe {
                let _ = self
                    .layout
                    .SetDrawingEffect(surface.brush(color), native_range(&span.range));
            }
        }
        self.chips = chips;
        self.decorated = true;
    }
}

pub struct Surface {
    target: ID2D1DCRenderTarget,
    brushes: Vec<ID2D1SolidColorBrush>,
}

impl Surface {
    fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        &self.brushes[color as usize]
    }

    pub fn bind(&self, dc: HDC, rect: &RECT, dpi: f32) -> Result<()> {
        unsafe {
            self.target.SetDpi(dpi, dpi);
            self.target.BindDC(dc, rect)
        }
    }

    pub fn begin(&self) {
        unsafe {
            self.target.BeginDraw();
            self.target.Clear(Some(&rgba(COLORS[Color::Page as usize])));
        }
    }

    pub fn end(&self) -> Result<()> {
        unsafe { self.target.EndDraw(None, None) }
    }

    pub fn canvas(&self, x: f32, y: f32) -> Canvas<'_> {
        Canvas {
            surface: self,
            x,
            y,
        }
    }
}

pub struct Canvas<'a> {
    pub surface: &'a Surface,
    x: f32,
    y: f32,
}

impl Canvas<'_> {
    fn offset(&self, mut rect: Rect) -> Rect {
        rect.left += self.x;
        rect.right += self.x;
        rect.top += self.y;
        rect.bottom += self.y;
        rect
    }

    pub fn clip(&self, rect: Rect) {
        unsafe {
            self.surface
                .target
                .PushAxisAlignedClip(&self.offset(rect), D2D1_ANTIALIAS_MODE_ALIASED);
        }
    }

    pub fn unclip(&self) {
        unsafe {
            self.surface.target.PopAxisAlignedClip();
        }
    }

    pub fn fill(&self, rect: Rect, color: Color) {
        unsafe {
            self.surface
                .target
                .FillRectangle(&self.offset(rect), self.surface.brush(color));
        }
    }

    pub fn round(&self, rect: Rect, radius: f32, color: Color) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: self.offset(rect),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.surface
                .target
                .FillRoundedRectangle(&rounded, self.surface.brush(color));
        }
    }

    pub fn line(&self, start: (f32, f32), end: (f32, f32), width: f32, color: Color) {
        unsafe {
            self.surface.target.DrawLine(
                Vector2 {
                    X: start.0 + self.x,
                    Y: start.1 + self.y,
                },
                Vector2 {
                    X: end.0 + self.x,
                    Y: end.1 + self.y,
                },
                self.surface.brush(color),
                width,
                None,
            );
        }
    }

    pub fn text(&self, glyphs: &Glyphs, x: f32, y: f32, color: Color, selection: Range<u32>) {
        for chip in &glyphs.chips {
            self.round(
                area(
                    x + chip.left - 1.5,
                    y + chip.top + 3.0,
                    chip.right - chip.left + 3.0,
                    (chip.bottom - chip.top - 6.0).max(1.0),
                ),
                3.0,
                Color::Surface,
            );
        }
        glyphs.ranges(selection, |rect| {
            self.fill(
                area(
                    x + rect.left,
                    y + rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ),
                Color::Selection,
            );
        });
        unsafe {
            self.surface.target.DrawTextLayout(
                Vector2 {
                    X: self.x + x,
                    Y: self.y + y,
                },
                &glyphs.layout,
                self.surface.brush(color),
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }
    }
}
