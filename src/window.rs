use crate::{
    associations,
    locale::{self, Messages},
    markdown::{self, Text},
    render::{Color, Face, Graphics, Rect, Surface, area},
    scroll::{AUTOSCROLL_DEAD_ZONE, WheelMotion, Zoom, autoscroll_speed},
    view::{Cursor, View, contains, local_path, url_text},
};
use pulldown_cmark::Alignment;
use std::{
    cell::RefCell,
    env,
    ffi::OsString,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{
            D2DERR_RECREATE_TARGET, GlobalFree, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
            RECT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
        },
        Graphics::{
            Dwm::{
                DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
                DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
            },
            Gdi::{
                BeginPaint, EndPaint, GetMonitorInfoW, HDC, InvalidateRect,
                MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
                ScreenToClient, UpdateWindow,
            },
        },
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            LibraryLoader::GetModuleHandleW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Threading::{
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CancelWaitableTimer, CreateWaitableTimerExW,
                INFINITE, SYNCHRONIZATION_SYNCHRONIZE, SetWaitableTimer, TIMER_MODIFY_STATE,
            },
        },
        UI::{
            HiDpi::{GetDpiForSystem, GetDpiForWindow},
            Input::KeyboardAndMouse::{
                GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN,
                VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_SHIFT, VK_SPACE, VK_UP,
            },
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP, ShellExecuteW},
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, Error, PCWSTR, Result, w},
};

const FRAME_MILLISECONDS: i32 = 16;
const SET_DEFAULT_COMMAND: usize = 0x0100;
const MK_CONTROL: usize = 0x0008;

thread_local! {
    static VIEWER: RefCell<Option<Viewer>> = const { RefCell::new(None) };
}

#[derive(Default)]
enum Drag {
    #[default]
    None,
    Text {
        x: f32,
        y: f32,
        link: Option<String>,
    },
    Scrollbar {
        offset: f32,
    },
    Autoscroll(Autoscroll),
}

struct Autoscroll {
    x: f32,
    y: f32,
    speed: f32,
    dragged: bool,
}

struct Viewer {
    hwnd: HWND,
    messages: &'static Messages,
    gfx: Graphics,
    surface: Option<Surface>,
    dpi: f32,
    width: f32,
    height: f32,
    scroll: f32,
    zoom: Zoom,
    document_scale: f32,
    zoom_anchor: f32,
    wheel_lines: u32,
    animate_scroll: bool,
    wheel_motion: Option<WheelMotion>,
    scroll_timer: OwnedHandle,
    scroll_tick: Option<Instant>,
    doc: Option<View>,
    notice: Option<String>,
    selection: Option<(Cursor, Cursor)>,
    drag: Drag,
    cursors: [HCURSOR; 4],
}

pub fn run() {
    let messages = locale::current();
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() };
    let result = result.and_then(|()| {
        let result = run_window(messages);
        VIEWER.with(|slot| {
            slot.borrow_mut().take();
        });
        unsafe {
            CoUninitialize();
        }
        result
    });
    if let Err(error) = result {
        show_error(
            None,
            &messages.system_error(messages.startup_failed, error.code().0 as u32),
        );
    }
}

fn show_error(window: Option<HWND>, text: &str) {
    let message: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            window,
            PCWSTR(message.as_ptr()),
            w!("mdrilo"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn run_window(messages: &'static Messages) -> Result<()> {
    let gfx = Graphics::new()?;
    unsafe {
        let timer_access = (TIMER_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE).0;
        let timer = CreateWaitableTimerExW(
            None,
            PCWSTR::null(),
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            timer_access,
        )
        .or_else(|_| CreateWaitableTimerExW(None, PCWSTR::null(), 0, timer_access))?;
        let scroll_timer = OwnedHandle::from_raw_handle(timer.0);
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = w!("MdriloViewer");
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        let text = LoadCursorW(None, IDC_IBEAM)?;
        let hand = LoadCursorW(None, IDC_HAND)?;
        let vertical = LoadCursorW(None, IDC_SIZENS)?;
        let icon = |width: i32| {
            LoadImageW(
                Some(instance),
                PCWSTR(ptr::without_provenance(1)),
                IMAGE_ICON,
                width,
                width,
                LR_SHARED,
            )
            .map_or(HICON::default(), |handle| HICON(handle.0))
        };
        let wndclass = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: arrow,
            hIcon: icon(32),
            hIconSm: icon(16),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wndclass) == 0 {
            return Err(Error::from_thread());
        }
        let bounds = initial_bounds();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("mdrilo"),
            WS_OVERLAPPEDWINDOW,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(instance),
            None,
        )?;
        dark_frame(hwnd);
        DragAcceptFiles(hwnd, true);
        let menu = GetSystemMenu(hwnd, false);
        let label: Vec<u16> = messages.set_default.encode_utf16().chain([0]).collect();
        let _ = InsertMenuW(
            menu,
            SC_CLOSE,
            MF_BYCOMMAND | MF_STRING,
            SET_DEFAULT_COMMAND,
            PCWSTR(label.as_ptr()),
        );
        let mut viewer = Viewer {
            hwnd,
            messages,
            gfx,
            surface: None,
            dpi: 96.0,
            width: 0.0,
            height: 0.0,
            scroll: 0.0,
            zoom: Zoom::default(),
            document_scale: 1.0,
            zoom_anchor: 0.0,
            wheel_lines: wheel_lines(),
            animate_scroll: scroll_animation_enabled(),
            wheel_motion: None,
            scroll_timer,
            scroll_tick: None,
            doc: None,
            notice: None,
            selection: None,
            drag: Drag::None,
            cursors: [arrow, text, hand, vertical],
        };
        if let Some(path) = env::args_os().nth(1) {
            viewer.open(PathBuf::from(path));
        }
        VIEWER.with(|slot| {
            *slot.borrow_mut() = Some(viewer);
        });
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = UpdateWindow(hwnd);
        let _ = associations::add_explorer_command(messages.open_with_mdrilo);
        let mut message = MSG::default();
        loop {
            let ready = MsgWaitForMultipleObjectsEx(
                Some(&[timer]),
                INFINITE,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
            if ready == WAIT_FAILED {
                return Err(Error::from_thread());
            }
            if ready == WAIT_OBJECT_0 {
                let changed = VIEWER.with(|slot| {
                    slot.borrow_mut()
                        .as_mut()
                        .is_some_and(Viewer::advance_scroll)
                });
                if changed {
                    let _ = UpdateWindow(hwnd);
                }
            }
            if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn initial_bounds() -> RECT {
    unsafe {
        let dpi = GetDpiForSystem().max(96) as f32 / 96.0;
        let width = (944.0 * dpi).round() as i32;
        let height = (730.0 * dpi).round() as i32;
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work = info.rcWork;
            let width = width.min(work.right - work.left);
            let height = height.min(work.bottom - work.top);
            let left = (work.left + work.right - width) / 2;
            let top = (work.top + work.bottom - height) / 2;
            RECT {
                left,
                top,
                right: left + width,
                bottom: top + height,
            }
        } else {
            RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            }
        }
    }
}

fn dark_frame(hwnd: HWND) {
    let set = |attribute, value: u32| unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            ptr::from_ref(&value).cast(),
            size_of::<u32>() as u32,
        )
    };
    if set(DWMWA_USE_IMMERSIVE_DARK_MODE, 1).is_err() {
        let _ = set(DWMWINDOWATTRIBUTE(19), 1);
    }
    let _ = set(DWMWA_CAPTION_COLOR, 0x282420);
    let _ = set(DWMWA_TEXT_COLOR, 0xfaf3f1);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = (344.0 * dpi) as i32;
            info.ptMinTrackSize.y = (292.0 * dpi) as i32;
            return LRESULT(0);
        }
        _ => (),
    }
    // Some Win32 calls synchronously send messages back to this window.
    let handled = VIEWER.with(|slot| {
        slot.try_borrow_mut()
            .ok()?
            .as_mut()?
            .message(message, wparam, lparam)
    });
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

impl Viewer {
    fn open(&mut self, path: PathBuf) {
        self.stop_wheel();
        self.stop_autoscroll();
        let path = std::path::absolute(&path).unwrap_or(path);
        let name = path.file_name().unwrap_or(path.as_os_str());
        let title: Vec<u16> = name
            .encode_wide()
            .chain(" — mdrilo".encode_utf16())
            .chain([0])
            .collect();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()));
        }
        self.doc = None;
        self.notice = None;
        self.selection = None;
        self.drag = Drag::None;
        self.scroll = 0.0;
        self.zoom_anchor = 0.0;
        match markdown::read(&path) {
            Ok(blocks) => {
                self.doc = Some(View::new(path, blocks, self.messages.image_unavailable));
            }
            Err(error) => self.notice = Some(self.messages.file_error(&error)),
        }
        self.redraw();
    }

    fn redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn column(&self) -> (f32, f32) {
        let available = self.width / self.document_scale;
        let padding = (self.width * 0.066).clamp(22.0, 62.0) / self.document_scale;
        let width = (available - padding * 2.0).clamp(1.0, 758.0);
        ((available - width) * 0.5, width)
    }

    fn scroll_max(&self) -> f32 {
        self.doc.as_ref().map_or(0.0, |doc| {
            (doc.height() * self.document_scale - self.height).max(0.0)
        })
    }

    fn scroll_to(&mut self, position: f32) {
        self.stop_wheel();
        self.stop_autoscroll();
        self.set_scroll(position);
    }

    fn set_scroll(&mut self, position: f32) -> bool {
        let previous = pixel_scroll(self.scroll, self.dpi);
        self.scroll = position.clamp(0.0, self.scroll_max());
        let changed = pixel_scroll(self.scroll, self.dpi) != previous;
        if changed {
            self.redraw();
        }
        changed
    }

    fn scroll_wheel(&mut self, delta: i16) {
        if delta == 0 || self.wheel_lines == 0 || matches!(self.drag, Drag::Scrollbar { .. }) {
            return;
        }
        self.stop_autoscroll();
        let step = (self.wheel_lines as f32 * 29.0).min(self.height * 0.88);
        let distance = -f32::from(delta) * step / WHEEL_DELTA as f32;
        let motion = WheelMotion::retarget(
            self.wheel_motion.as_ref(),
            self.scroll,
            distance,
            self.scroll_max(),
        );
        if !self.animate_scroll || motion.target == self.scroll {
            self.scroll_to(motion.target);
            return;
        }
        self.wheel_motion = Some(motion);
        self.sync_scroll_timer();
    }

    fn zoom_wheel(&mut self, delta: i16, y: f32) {
        if self.doc.as_ref().is_some_and(View::has_content)
            && !matches!(self.drag, Drag::Scrollbar { .. })
            && self.zoom.wheel(delta)
        {
            self.stop_wheel();
            self.stop_autoscroll();
            self.zoom_anchor = y;
            self.redraw();
        }
    }

    fn advance_scroll(&mut self) -> bool {
        let Some(previous) = self.scroll_tick else {
            return false;
        };
        let now = Instant::now();
        let seconds = (now - previous).as_secs_f32();
        self.scroll_tick = Some(now);
        let position = if let Some(motion) = &mut self.wheel_motion {
            let position = motion.advance(self.scroll, seconds);
            if position == motion.target {
                self.wheel_motion = None;
            }
            position
        } else if let Drag::Autoscroll(auto) = &self.drag {
            self.scroll + auto.speed * seconds.min(0.05)
        } else {
            self.scroll
        };
        let changed = self.set_scroll(position);
        self.sync_scroll_timer();
        changed
    }

    fn sync_scroll_timer(&mut self) {
        let needs_frames = self.wheel_motion.is_some()
            || matches!(&self.drag, Drag::Autoscroll(auto)
            if (auto.speed < 0.0 && self.scroll > 0.0) || (auto.speed > 0.0 && self.scroll < self.scroll_max()));
        if needs_frames == self.scroll_tick.is_some() {
            return;
        }
        let timer = HANDLE(self.scroll_timer.as_raw_handle());
        if needs_frames {
            let due = -i64::from(FRAME_MILLISECONDS) * 10_000;
            if unsafe { SetWaitableTimer(timer, &due, FRAME_MILLISECONDS, None, None, false) }
                .is_ok()
            {
                self.scroll_tick = Some(Instant::now());
            } else if let Some(motion) = self.wheel_motion.take() {
                self.set_scroll(motion.target);
            }
        } else {
            self.scroll_tick = None;
            unsafe {
                let _ = CancelWaitableTimer(timer);
            }
        }
    }

    fn stop_wheel(&mut self) {
        if self.wheel_motion.take().is_some() {
            self.sync_scroll_timer();
        }
    }

    fn thumb(&self) -> Option<Rect> {
        let maximum = self.scroll_max();
        if maximum <= 0.0 || self.height <= 40.0 {
            return None;
        }
        let track = self.height - 20.0;
        let height = (track * self.height / (self.height + maximum)).clamp(28.0, track);
        let top = 10.0 + (track - height) * self.scroll / maximum;
        Some(area(self.width - 10.0, top, 5.0, height))
    }

    fn drop_surface(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.clear_device_resources();
        }
        self.surface = None;
    }

    fn paint(&mut self, dc: HDC) {
        let mut client = RECT::default();
        if dc.is_invalid()
            || unsafe { GetClientRect(self.hwnd, &mut client) }.is_err()
            || client.right <= 0
            || client.bottom <= 0
        {
            return;
        }
        self.dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32;
        self.width = client.right as f32 * 96.0 / self.dpi;
        self.height = client.bottom as f32 * 96.0 / self.dpi;
        if self.surface.is_none() {
            self.surface = self.gfx.surface().ok();
        }
        let Some(surface) = &self.surface else { return };
        if surface.bind(dc, &client, self.dpi).is_err() {
            self.drop_surface();
            return;
        }
        let previous_scale = self.document_scale;
        self.document_scale = self.zoom.scale();
        let scale = self.document_scale;
        let anchor_y = if scale == previous_scale {
            0.0
        } else {
            self.zoom_anchor.clamp(0.0, self.height)
        };
        let (left, width) = self.column();
        surface.begin();
        if let Some(doc) = &mut self.doc
            && doc.has_content()
        {
            let anchor = doc.reflow(&self.gfx, width, (self.scroll + anchor_y) / previous_scale);
            self.scroll = (anchor * scale - anchor_y)
                .clamp(0.0, (doc.height() * scale - self.height).max(0.0));
            let visible_scroll = pixel_scroll(self.scroll, self.dpi) / scale;
            let viewport_height = self.height / scale;
            surface.scaled(scale, || {
                let canvas = surface.canvas(left, -visible_scroll);
                canvas.clip(area(-10.0, visible_scroll, width + 20.0, viewport_height));
                doc.paint(
                    &self.gfx,
                    &canvas,
                    visible_scroll,
                    viewport_height,
                    self.dpi * scale / 96.0,
                    self.selection,
                );
                canvas.unclip();
            });
            if let Some(thumb) = self.thumb() {
                let color = if matches!(self.drag, Drag::Scrollbar { .. }) {
                    Color::Quote
                } else {
                    Color::Thumb
                };
                surface.canvas(0.0, 0.0).round(thumb, 2.5, color);
            }
        } else {
            self.paint_notice(surface);
        }
        if let Drag::Autoscroll(auto) = &self.drag {
            let marker = surface.canvas(auto.x, auto.y);
            marker.round(area(-13.0, -13.0, 26.0, 26.0), 13.0, Color::Quote);
            marker.round(area(-12.0, -12.0, 24.0, 24.0), 12.0, Color::Surface);
            for direction in [-1.0, 1.0] {
                let tip = (0.0, direction * 7.5);
                marker.line((-3.5, direction * 4.5), tip, 1.5, Color::Accent);
                marker.line(tip, (3.5, direction * 4.5), 1.5, Color::Accent);
            }
            marker.round(area(-1.15, -1.15, 2.3, 2.3), 1.15, Color::Accent);
        }
        if let Err(error) = surface.end() {
            self.drop_surface();
            if error.code() == D2DERR_RECREATE_TARGET {
                self.redraw();
            }
        }
    }

    fn paint_notice(&self, surface: &Surface) {
        let (heading, details) = match self.notice.as_deref() {
            Some(details) => (self.messages.open_failed, details),
            None if self.doc.is_some() => (self.messages.empty_file, ""),
            None => (self.messages.drop_title, self.messages.drop_hint),
        };
        let width = (self.width - 36.0).clamp(1.0, 490.0);
        let top = ((self.height - 135.0) * 0.46).max(14.0);
        let canvas = surface.canvas((self.width - width) * 0.5, top);
        let x = (width - 52.0) * 0.5;
        canvas.round(area(x, 0.0, 52.0, 52.0), 14.0, Color::AccentSurface);
        let lines = [
            ((12.0, 34.0), (12.0, 18.0)),
            ((12.0, 18.0), (18.0, 24.0)),
            ((18.0, 24.0), (24.0, 18.0)),
            ((24.0, 18.0), (24.0, 34.0)),
            ((36.0, 18.0), (36.0, 34.0)),
            ((30.0, 28.0), (36.0, 34.0)),
            ((36.0, 34.0), (42.0, 28.0)),
        ];
        for (from, to) in lines {
            canvas.line((x + from.0, from.1), (x + to.0, to.1), 2.6, Color::Accent);
        }
        if let Ok(glyphs) = self
            .gfx
            .text(&Text::new(heading), Face::H3, Alignment::Center, width)
        {
            canvas.text(&glyphs, 0.0, 68.0, Color::Heading, 0..0);
            if !details.is_empty()
                && let Ok(details) =
                    self.gfx
                        .text(&Text::new(details), Face::Small, Alignment::Center, width)
            {
                canvas.text(&details, 0.0, 74.0 + glyphs.height, Color::Muted, 0..0);
            }
        }
    }

    fn point(&self, lparam: LPARAM) -> (f32, f32) {
        let point = message_point(lparam);
        (
            point.x as f32 * 96.0 / self.dpi,
            point.y as f32 * 96.0 / self.dpi,
        )
    }

    fn document_point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x / self.document_scale - self.column().0,
            (y + pixel_scroll(self.scroll, self.dpi)) / self.document_scale,
        )
    }

    fn cursor_at(&mut self, x: f32, y: f32) -> HCURSOR {
        if matches!(self.drag, Drag::Autoscroll(_)) {
            return self.cursors[3];
        }
        if x >= self.width - 17.0 && self.thumb().is_some() {
            return self.cursors[0];
        }
        let (x, y) = self.document_point(x, y);
        let Some(doc) = &mut self.doc else {
            return self.cursors[0];
        };
        let hit = doc.hit(&self.gfx, x, y);
        if hit.link.is_some() {
            self.cursors[2]
        } else if hit.text {
            self.cursors[1]
        } else {
            self.cursors[0]
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        self.stop_wheel();
        if x >= self.width - 17.0
            && let Some(thumb) = self.thumb()
        {
            if contains(
                area(self.width - 17.0, thumb.top, 17.0, thumb.bottom - thumb.top),
                x,
                y,
            ) {
                self.drag = Drag::Scrollbar {
                    offset: y - thumb.top,
                };
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.redraw();
            } else {
                let direction = if y < thumb.top { -1.0 } else { 1.0 };
                self.scroll_to(self.scroll + direction * self.height * 0.88);
            }
            return;
        }
        let (doc_x, doc_y) = self.document_point(x, y);
        let Some(doc) = &mut self.doc else { return };
        let hit = doc.hit(&self.gfx, doc_x, doc_y);
        self.drag = Drag::Text {
            x,
            y,
            link: hit.link.map(str::to_owned),
        };
        self.selection = hit.cursor.map(|cursor| {
            let anchor = if key_down(VK_SHIFT.0) {
                self.selection.map_or(cursor, |(anchor, _)| anchor)
            } else {
                cursor
            };
            (anchor, cursor)
        });
        unsafe {
            SetCapture(self.hwnd);
        }
        self.redraw();
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::Text { .. } => {
                if y < 0.0 || y > self.height {
                    let distance = y - y.clamp(0.0, self.height);
                    self.scroll_to(self.scroll + distance.clamp(-95.0, 95.0));
                }
                let (doc_x, doc_y) = self.document_point(x, y.clamp(0.0, self.height));
                if let Some(doc) = &mut self.doc
                    && let Some((start, end)) = self.selection
                    && let Some(cursor) = doc.hit(&self.gfx, doc_x, doc_y).cursor
                    && cursor != end
                {
                    self.selection = Some((start, cursor));
                    self.redraw();
                }
            }
            Drag::Scrollbar { offset } => {
                if let Some(thumb) = self.thumb() {
                    let travel = self.height - 20.0 - (thumb.bottom - thumb.top);
                    self.scroll_to(self.scroll_max() * (y - 10.0 - offset) / travel.max(1.0));
                }
            }
            Drag::Autoscroll(ref mut auto) => {
                auto.speed = autoscroll_speed(y - auto.y);
                auto.dragged |= auto.speed != 0.0;
                self.sync_scroll_timer();
            }
            Drag::None => (),
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let drag = std::mem::take(&mut self.drag);
        if !matches!(drag, Drag::None) {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        match drag {
            Drag::Text {
                x: start_x,
                y: start_y,
                link: Some(link),
            } if (x - start_x).abs() < 4.0
                && (y - start_y).abs() < 4.0
                && self.selection.is_none_or(|(from, to)| from == to) =>
            {
                self.follow(&link)
            }
            Drag::Scrollbar { .. } => self.redraw(),
            _ => (),
        }
    }

    fn middle_down(&mut self, x: f32, y: f32) {
        if self.stop_autoscroll() || !matches!(self.drag, Drag::None) || self.scroll_max() <= 0.0 {
            return;
        }
        self.stop_wheel();
        self.drag = Drag::Autoscroll(Autoscroll {
            x,
            y,
            speed: 0.0,
            dragged: false,
        });
        unsafe {
            SetCapture(self.hwnd);
            SetCursor(Some(self.cursors[3]));
        }
        self.redraw();
    }

    fn middle_up(&mut self, y: f32) {
        if matches!(&self.drag, Drag::Autoscroll(auto) if auto.dragged || (y - auto.y).abs() > AUTOSCROLL_DEAD_ZONE)
        {
            self.stop_autoscroll();
        }
    }

    fn stop_autoscroll(&mut self) -> bool {
        if !matches!(self.drag, Drag::Autoscroll(_)) {
            return false;
        }
        self.drag = Drag::None;
        self.sync_scroll_timer();
        unsafe {
            if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
                SetCursor(Some(self.cursors[0]));
            }
        }
        self.redraw();
        true
    }

    fn follow(&mut self, url: &str) {
        if let Some(fragment) = url.strip_prefix('#') {
            let position = url_text(fragment).and_then(|anchor| self.doc.as_ref()?.anchor(&anchor));
            if let Some(position) = position {
                self.scroll_to(position * self.document_scale);
            }
            return;
        }
        if let Some((scheme, _)) = url.split_once(':')
            && ["https", "http", "mailto"]
                .iter()
                .any(|supported| scheme.eq_ignore_ascii_case(supported))
            && !url.chars().any(char::is_control)
        {
            let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
            unsafe {
                ShellExecuteW(
                    Some(self.hwnd),
                    w!("open"),
                    PCWSTR(wide.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
            }
            return;
        }
        if let Some(path) = self
            .doc
            .as_ref()
            .and_then(|doc| local_path(&doc.path, url))
            .filter(|path| is_markdown(path))
        {
            self.open(path);
            if let Some((_, hash)) = url.split_once('#')
                && let Some(anchor) = url_text(hash)
            {
                let width = self.column().1;
                if let Some(doc) = &mut self.doc {
                    doc.reflow(&self.gfx, width, 0.0);
                    if let Some(position) = doc.anchor(&anchor) {
                        self.scroll_to(position * self.document_scale);
                    }
                }
            }
        }
    }

    fn key(&mut self, key: u16) -> bool {
        match key {
            65 if key_down(VK_CONTROL.0) => {
                self.selection = self.doc.as_ref().and_then(View::all);
                self.redraw();
            }
            67 if key_down(VK_CONTROL.0) => {
                if let (Some(doc), Some(selection)) = (&self.doc, self.selection) {
                    clipboard(self.hwnd, doc.copy(selection));
                }
            }
            k if k == VK_UP.0 => self.scroll_to(self.scroll - 38.0),
            k if k == VK_DOWN.0 => self.scroll_to(self.scroll + 38.0),
            k if k == VK_PRIOR.0 => self.scroll_to(self.scroll - self.height * 0.88),
            k if k == VK_NEXT.0 => self.scroll_to(self.scroll + self.height * 0.88),
            k if k == VK_SPACE.0 => {
                let direction = if key_down(VK_SHIFT.0) { -1.0 } else { 1.0 };
                self.scroll_to(self.scroll + direction * self.height * 0.88);
            }
            k if k == VK_HOME.0 => self.scroll_to(0.0),
            k if k == VK_END.0 => self.scroll_to(self.scroll_max()),
            k if k == VK_ESCAPE.0 => {
                if !self.stop_autoscroll() {
                    self.selection = None;
                    self.redraw();
                }
            }
            _ => return false,
        }
        true
    }

    fn message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_SYSCOMMAND if wparam.0 & 0xfff0 == SET_DEFAULT_COMMAND => {
                if let Err(error) =
                    associations::open_default_settings(self.hwnd, self.messages.app_description)
                {
                    let message = self
                        .messages
                        .system_error(self.messages.default_settings_failed, error.code().0 as u32);
                    show_error(Some(self.hwnd), &message);
                }
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = unsafe { BeginPaint(self.hwnd, &mut paint) };
                self.paint(dc);
                unsafe {
                    let _ = EndPaint(self.hwnd, &paint);
                }
            }
            WM_PRINTCLIENT => self.paint(HDC(wparam.0 as *mut _)),
            WM_SIZE => {
                self.stop_wheel();
                self.stop_autoscroll();
                self.redraw();
            }
            WM_DPICHANGED => {
                self.stop_wheel();
                self.stop_autoscroll();
                let bounds = unsafe { &*(lparam.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        bounds.left,
                        bounds.top,
                        bounds.right - bounds.left,
                        bounds.bottom - bounds.top,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
                self.redraw();
            }
            WM_ACTIVATE if wparam.0 as u16 == WA_INACTIVE as u16 => {
                self.stop_autoscroll();
                return None;
            }
            WM_SETTINGCHANGE => {
                self.wheel_lines = wheel_lines();
                self.animate_scroll = scroll_animation_enabled();
                if !self.animate_scroll
                    && let Some(motion) = &self.wheel_motion
                {
                    self.scroll_to(motion.target);
                }
                dark_frame(self.hwnd);
            }
            WM_DROPFILES => {
                if let Some(path) = dropped(HDROP(wparam.0 as *mut _)) {
                    self.open(path);
                    unsafe {
                        let _ = SetFocus(Some(self.hwnd));
                    }
                }
            }
            WM_MOUSEWHEEL => {
                let delta = (wparam.0 >> 16) as i16;
                if wparam.0 & MK_CONTROL != 0 {
                    let mut point = message_point(lparam);
                    unsafe {
                        let _ = ScreenToClient(self.hwnd, &mut point);
                    }
                    self.zoom_wheel(delta, point.y as f32 * 96.0 / self.dpi);
                } else {
                    self.scroll_wheel(delta);
                }
            }
            WM_MBUTTONDOWN => {
                let (x, y) = self.point(lparam);
                self.middle_down(x, y);
            }
            WM_MBUTTONUP => self.middle_up(self.point(lparam).1),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN if self.stop_autoscroll() => (),
            WM_XBUTTONDOWN if self.stop_autoscroll() => return Some(LRESULT(1)),
            WM_LBUTTONDOWN => {
                let (x, y) = self.point(lparam);
                self.mouse_down(x, y);
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.point(lparam);
                self.mouse_move(x, y);
            }
            WM_LBUTTONUP if !matches!(self.drag, Drag::Autoscroll(_)) => {
                let (x, y) = self.point(lparam);
                self.mouse_up(x, y);
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                if !self.stop_autoscroll() && !matches!(self.drag, Drag::None) {
                    self.drag = Drag::None;
                    self.redraw();
                }
                if message == WM_CANCELMODE {
                    return None;
                }
            }
            WM_SETCURSOR if lparam.0 as u16 == HTCLIENT as u16 => unsafe {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = ScreenToClient(self.hwnd, &mut point);
                let cursor = self.cursor_at(
                    point.x as f32 * 96.0 / self.dpi,
                    point.y as f32 * 96.0 / self.dpi,
                );
                SetCursor(Some(cursor));
                return Some(LRESULT(1));
            },
            WM_KEYDOWN if self.key(wparam.0 as u16) => (),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

fn message_point(lparam: LPARAM) -> POINT {
    let bits = lparam.0 as u32;
    POINT {
        x: i32::from(bits as i16),
        y: i32::from((bits >> 16) as i16),
    }
}

fn pixel_scroll(position: f32, dpi: f32) -> f32 {
    let scale = dpi / 96.0;
    (position * scale).round() / scale
}

fn wheel_lines() -> u32 {
    let mut lines = 3u32;
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(ptr::from_mut(&mut lines).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        );
    }
    lines
}

fn scroll_animation_enabled() -> bool {
    let mut enabled = BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(ptr::from_mut(&mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        );
    }
    enabled.as_bool()
}

fn key_down(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

fn dropped(handle: HDROP) -> Option<PathBuf> {
    unsafe {
        let count = DragQueryFileW(handle, u32::MAX, None);
        let mut first = None;
        for index in 0..count {
            let length = DragQueryFileW(handle, index, None) as usize;
            let mut name = vec![0; length + 1];
            if DragQueryFileW(handle, index, Some(&mut name)) as usize == length && length > 0 {
                let path = PathBuf::from(OsString::from_wide(&name[..length]));
                if is_markdown(&path) {
                    first = Some(path);
                    break;
                }
                if first.is_none() {
                    first = Some(path);
                }
            }
        }
        DragFinish(handle);
        first
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["md", "markdown", "mdown", "mkd", "mkdn"]
            .iter()
            .any(|value| ext.eq_ignore_ascii_case(value))
    })
}

fn clipboard(hwnd: HWND, mut text: Vec<u16>) {
    if text.is_empty() {
        return;
    }
    text.push(0);
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, text.len() * size_of::<u16>()) else {
            return;
        };
        let destination = GlobalLock(memory).cast::<u16>();
        if destination.is_null() {
            let _ = GlobalFree(Some(memory));
            return;
        }
        ptr::copy_nonoverlapping(text.as_ptr(), destination, text.len());
        let _ = GlobalUnlock(memory);
        let copied = if OpenClipboard(Some(hwnd)).is_ok() {
            let success = EmptyClipboard()
                .and_then(|()| SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))))
                .is_ok();
            let _ = CloseClipboard();
            success
        } else {
            false
        };
        if !copied {
            let _ = GlobalFree(Some(memory));
        }
    }
}
