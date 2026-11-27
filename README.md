# mdrilo

Markdown viewer for Windows x64. [Download](https://github.com/qventymr/mdrilo/releases/latest)

Drop a `.md` file into the window or use **Open with MDRilo** in Explorer after first launch. On Windows 11, use **Show more options**.

Middle-click to scroll by moving the mouse.
Hold **Ctrl** and turn the mouse wheel to zoom.
Use **Set as default for .md…** in the title bar menu to open Windows’ default app settings.

## Build

Requires Rust (MSVC), Visual Studio C++ Build Tools, and the Windows SDK.

```powershell
cargo build --release --locked
```

The executable is `target\release\mdrilo.exe`.
