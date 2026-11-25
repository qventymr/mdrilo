use std::{
    env,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{ERROR_GEN_FAILURE, ERROR_SUCCESS, HWND},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_CREATE_SUB_KEY, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_NONE,
            REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
            RegGetValueW, RegSetValueExW,
        },
        UI::{
            Shell::{
                SEE_MASK_FLAG_NO_UI, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
                SHELLEXECUTEINFOW, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{HRESULT, PCWSTR, Result, w},
};

const APP_NAME: &str = "mdrilo";
const PROG_ID: &str = "mdrilo.Markdown";
const CAPABILITIES: &str = r"Software\mdrilo\Capabilities";
const EXPLORER_VERB: PCWSTR = w!(r"Software\Classes\SystemFileAssociations\.md\shell\mdrilo.open");

pub fn add_explorer_command(label: &str) -> Result<()> {
    if register_explorer_command(HKEY_CURRENT_USER, &current_executable()?, label)? {
        notify_shell();
    }
    Ok(())
}

fn current_executable() -> Result<PathBuf> {
    env::current_exe().map_err(|error| {
        HRESULT::from_win32(
            error
                .raw_os_error()
                .map_or(ERROR_GEN_FAILURE.0, |code| code as u32),
        )
        .into()
    })
}

fn notify_shell() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

pub fn open_default_settings(window: HWND, description: &str) -> Result<()> {
    register(HKEY_CURRENT_USER, &current_executable()?, description)?;
    notify_shell();

    let launch = |uri| {
        let mut settings = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI,
            hwnd: window,
            lpVerb: w!("open"),
            lpFile: uri,
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut settings) }
    };
    launch(w!("ms-settings:defaultapps?registeredAppUser=mdrilo"))
        .or_else(|_| launch(w!("ms-settings:defaultapps")))
}

fn quoted_path(path: &Path) -> Vec<u16> {
    let quote = u16::from(b'"');
    [quote]
        .into_iter()
        .chain(path.as_os_str().encode_wide())
        .chain([quote])
        .collect()
}

fn register_explorer_command(root: HKEY, executable: &Path, label: &str) -> Result<bool> {
    let quoted_exe = quoted_path(executable);
    let verb = Key::create(root, EXPLORER_VERB)?;
    let mut changed = verb.text(w!("MUIVerb"), label.encode_utf16())?;
    changed |= verb.text(
        w!("Icon"),
        quoted_exe.iter().copied().chain(",0".encode_utf16()),
    )?;
    changed |= verb.text(w!("NeverDefault"), std::iter::empty())?;
    changed |= Key::create(verb.0, w!("command"))?.text(
        PCWSTR::null(),
        quoted_exe.iter().copied().chain(" \"%1\"".encode_utf16()),
    )?;
    Ok(changed)
}

fn register(root: HKEY, executable: &Path, description: &str) -> Result<()> {
    let quoted_exe = quoted_path(executable);
    let icon = quoted_exe.iter().copied().chain(",0".encode_utf16());

    let class = Key::create(root, w!(r"Software\Classes\mdrilo.Markdown"))?;
    class.text(PCWSTR::null(), "Markdown".encode_utf16())?;
    Key::create(class.0, w!("DefaultIcon"))?.text(PCWSTR::null(), icon.clone())?;
    Key::create(class.0, w!(r"shell\open\command"))?.text(
        PCWSTR::null(),
        quoted_exe.iter().copied().chain(" \"%1\"".encode_utf16()),
    )?;
    let application = Key::create(class.0, w!("Application"))?;
    application.text(w!("ApplicationName"), APP_NAME.encode_utf16())?;

    let capabilities = Key::create(root, w!(r"Software\mdrilo\Capabilities"))?;
    capabilities.text(w!("ApplicationName"), APP_NAME.encode_utf16())?;
    capabilities.text(w!("ApplicationDescription"), description.encode_utf16())?;
    capabilities.text(w!("ApplicationIcon"), icon)?;
    Key::create(capabilities.0, w!("FileAssociations"))?.text(w!(".md"), PROG_ID.encode_utf16())?;

    let candidates = Key::create(root, w!(r"Software\Classes\.md\OpenWithProgids"))?;
    unsafe {
        RegSetValueExW(candidates.0, w!("mdrilo.Markdown"), None, REG_NONE, None).ok()?;
    }
    Key::create(root, w!(r"Software\RegisteredApplications"))?
        .text(w!("mdrilo"), CAPABILITIES.encode_utf16())?;
    Ok(())
}

struct Key(HKEY);

impl Key {
    fn create(parent: HKEY, name: PCWSTR) -> Result<Self> {
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                parent,
                name,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_CREATE_SUB_KEY | KEY_QUERY_VALUE,
                None,
                &mut key,
                None,
            )
            .ok()?;
        }
        Ok(Self(key))
    }

    fn text(&self, name: PCWSTR, text: impl Iterator<Item = u16>) -> Result<bool> {
        let bytes: Vec<_> = text.chain([0]).flat_map(u16::to_le_bytes).collect();
        let mut existing = vec![0; bytes.len()];
        let mut size = existing.len() as u32;
        let status = unsafe {
            RegGetValueW(
                self.0,
                PCWSTR::null(),
                name,
                RRF_RT_REG_SZ,
                None,
                Some(existing.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status == ERROR_SUCCESS && size as usize == bytes.len() && existing == bytes {
            return Ok(false);
        }
        unsafe {
            RegSetValueExW(self.0, name, None, REG_SZ, Some(&bytes)).ok()?;
        }
        Ok(true)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, mem::size_of_val, ptr};
    use windows::Win32::System::Registry::{
        KEY_ALL_ACCESS, REG_PROCESS_APPKEY, RRF_RT_REG_NONE, RegLoadAppKeyW,
    };

    fn value(root: HKEY, path: PCWSTR, name: PCWSTR) -> String {
        let mut units = [0u16; 512];
        let mut size = size_of_val(&units) as u32;
        unsafe {
            RegGetValueW(
                root,
                path,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(units.as_mut_ptr().cast()),
                Some(&mut size),
            )
            .ok()
            .unwrap();
        }
        String::from_utf16(&units[..size as usize / 2 - 1]).unwrap()
    }

    #[test]
    fn registers_md_commands_and_keeps_the_existing_default() {
        let scratch = env::temp_dir().join(format!("mdrilo-associations-{}", std::process::id()));
        fs::create_dir(&scratch).unwrap();
        let file = scratch.join("sandbox.hiv");
        let name: Vec<u16> = file.as_os_str().encode_wide().chain([0]).collect();
        let mut root = HKEY::default();
        unsafe {
            RegLoadAppKeyW(
                PCWSTR(name.as_ptr()),
                &mut root,
                KEY_ALL_ACCESS.0,
                REG_PROCESS_APPKEY,
                None,
            )
            .ok()
            .unwrap();
        }
        let hive = Key(root);
        let extension = w!(r"Software\Classes\.md");
        Key::create(hive.0, extension)
            .unwrap()
            .text(PCWSTR::null(), "Existing.Editor".encode_utf16())
            .unwrap();
        let first_path = Path::new(r"C:\Portable Apps\mdrilo.exe");
        let russian = "Открыть с помощью MDRilo";
        assert!(register_explorer_command(hive.0, first_path, russian).unwrap());
        assert!(!register_explorer_command(hive.0, first_path, russian).unwrap());
        assert_eq!(value(hive.0, EXPLORER_VERB, w!("MUIVerb")), russian);
        assert_eq!(value(hive.0, EXPLORER_VERB, w!("NeverDefault")), "");
        let explorer_command =
            w!(r"Software\Classes\SystemFileAssociations\.md\shell\mdrilo.open\command");
        assert_eq!(
            value(hive.0, explorer_command, PCWSTR::null()),
            r#""C:\Portable Apps\mdrilo.exe" "%1""#
        );
        let moved_path = Path::new(r"D:\Новая папка\mdrilo.exe");
        let english = "Open with MDRilo";
        assert!(register_explorer_command(hive.0, moved_path, english).unwrap());
        assert_eq!(value(hive.0, EXPLORER_VERB, w!("MUIVerb")), english);
        assert_eq!(
            value(hive.0, explorer_command, PCWSTR::null()),
            r#""D:\Новая папка\mdrilo.exe" "%1""#
        );
        assert_eq!(value(hive.0, extension, PCWSTR::null()), "Existing.Editor");
        register(hive.0, first_path, "Markdown viewer").unwrap();
        assert_eq!(value(hive.0, extension, PCWSTR::null()), "Existing.Editor");
        let command = w!(r"Software\Classes\mdrilo.Markdown\shell\open\command");
        assert_eq!(
            value(hive.0, command, PCWSTR::null()),
            r#""C:\Portable Apps\mdrilo.exe" "%1""#
        );
        assert_eq!(
            value(hive.0, w!(r"Software\RegisteredApplications"), w!("mdrilo")),
            CAPABILITIES
        );
        assert_eq!(
            value(
                hive.0,
                w!(r"Software\mdrilo\Capabilities\FileAssociations"),
                w!(".md")
            ),
            PROG_ID
        );
        let mut bytes = 0;
        unsafe {
            RegGetValueW(
                hive.0,
                w!(r"Software\Classes\.md\OpenWithProgids"),
                w!("mdrilo.Markdown"),
                RRF_RT_REG_NONE,
                None,
                None,
                Some(ptr::from_mut(&mut bytes)),
            )
            .ok()
            .unwrap();
        }
        assert_eq!(bytes, 0);
        register(hive.0, moved_path, "Просмотрщик Markdown").unwrap();
        assert_eq!(
            value(hive.0, command, PCWSTR::null()),
            r#""D:\Новая папка\mdrilo.exe" "%1""#
        );
        assert_eq!(value(hive.0, extension, PCWSTR::null()), "Existing.Editor");
        drop(hive);
        for entry in fs::read_dir(&scratch).unwrap().flatten() {
            let _ = fs::remove_file(entry.path());
        }
        let _ = fs::remove_dir(scratch);
    }
}
