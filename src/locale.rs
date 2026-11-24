use std::io::{Error, ErrorKind};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

pub struct Messages {
    pub drop_title: &'static str,
    pub drop_hint: &'static str,
    pub empty_file: &'static str,
    pub open_failed: &'static str,
    pub image_unavailable: &'static str,
    startup_failed: &'static str,
    error_code: &'static str,
    file_not_found: &'static str,
    access_denied: &'static str,
    path_is_directory: &'static str,
    invalid_encoding: &'static str,
}

impl Messages {
    pub fn startup_error(&self, code: u32) -> String {
        format!(
            "{}\r\n{} 0x{code:08X}",
            self.startup_failed, self.error_code
        )
    }

    pub fn file_error(&self, error: &Error) -> String {
        match error.kind() {
            ErrorKind::NotFound => self.file_not_found.into(),
            ErrorKind::PermissionDenied => self.access_denied.into(),
            ErrorKind::IsADirectory => self.path_is_directory.into(),
            ErrorKind::InvalidData => self.invalid_encoding.into(),
            _ => error
                .raw_os_error()
                .map_or_else(String::new, |code| format!("{} {code}", self.error_code)),
        }
    }
}

const ENGLISH: Messages = Messages {
    drop_title: "Drag a Markdown file here",
    drop_hint: "Or choose mdrilo from “Open with”",
    empty_file: "Empty file",
    open_failed: "Couldn’t open the file",
    image_unavailable: "Image unavailable",
    startup_failed: "Couldn’t start mdrilo.",
    error_code: "Error code",
    file_not_found: "File not found.",
    access_denied: "Access to the file was denied.",
    path_is_directory: "This path points to a folder.",
    invalid_encoding: "Save the file as UTF-8 or UTF-16 with a BOM.",
};

const RUSSIAN: Messages = Messages {
    drop_title: "Перетащите Markdown-файл",
    drop_hint: "Или выберите mdrilo в меню «Открыть с помощью»",
    empty_file: "Файл пуст",
    open_failed: "Не удалось открыть файл",
    image_unavailable: "Изображение недоступно",
    startup_failed: "Не удалось запустить mdrilo.",
    error_code: "Код ошибки",
    file_not_found: "Файл не найден.",
    access_denied: "Доступ к файлу запрещён.",
    path_is_directory: "Путь указывает на папку.",
    invalid_encoding: "Сохраните файл в UTF-8 или UTF-16 с BOM.",
};

pub fn current() -> &'static Messages {
    for_language_id(unsafe { GetUserDefaultUILanguage() })
}

fn for_language_id(language: u16) -> &'static Messages {
    const PRIMARY_LANGUAGE_MASK: u16 = 0x03ff;
    const RUSSIAN_LANGUAGE_ID: u16 = 0x19;
    if language & PRIMARY_LANGUAGE_MASK == RUSSIAN_LANGUAGE_ID {
        &RUSSIAN
    } else {
        &ENGLISH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_russian_for_russian_ui_and_english_for_other_languages() {
        for language in [0x0419, 0x0819] {
            assert_eq!(for_language_id(language).drop_title, RUSSIAN.drop_title);
        }
        for language in [0x0409, 0x0809, 0x0407, 0x0422, 0] {
            assert_eq!(for_language_id(language).drop_title, ENGLISH.drop_title);
        }
    }

    #[test]
    fn errors_use_the_selected_language() {
        let missing = Error::from_raw_os_error(2);
        assert_eq!(ENGLISH.file_error(&missing), "File not found.");
        assert_eq!(RUSSIAN.file_error(&missing), "Файл не найден.");
        let invalid = Error::from(ErrorKind::InvalidData);
        assert_eq!(ENGLISH.file_error(&invalid), ENGLISH.invalid_encoding);
        assert_eq!(RUSSIAN.file_error(&invalid), RUSSIAN.invalid_encoding);
        assert!(ENGLISH.startup_error(5).ends_with("Error code 0x00000005"));
        assert!(RUSSIAN.startup_error(5).ends_with("Код ошибки 0x00000005"));
    }
}
