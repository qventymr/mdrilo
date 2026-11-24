# mdrilo

Markdown viewer for Windows x64. Drop a `.md` file into the window or use **Open with**.

Middle-click to scroll by moving the mouse.

## Build

Requires Rust (MSVC), Visual Studio C++ Build Tools, and the Windows SDK.

```powershell
cargo build --release --locked
```

The executable is `target\release\mdrilo.exe`.
