use crate::markdown::{Style, Text};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Plain,
    Rust,
    Python,
    JavaScript,
    Json,
    Config,
    CLike,
    Shell,
    PowerShell,
    Markup,
    Css,
    Sql,
}

#[derive(Clone, Copy)]
pub enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Name,
}

impl Language {
    pub fn from_info(info: &str) -> Self {
        match info
            .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "rs" | "rust" => Self::Rust,
            "py" | "python" | "python3" => Self::Python,
            "js" | "javascript" | "ts" | "typescript" | "jsx" | "tsx" => Self::JavaScript,
            "json" | "jsonc" => Self::Json,
            "yaml" | "yml" | "toml" | "ini" => Self::Config,
            "c" | "cpp" | "c++" | "cxx" | "cs" | "csharp" | "java" | "go" | "zig" => Self::CLike,
            "sh" | "bash" | "shell" | "zsh" | "fish" => Self::Shell,
            "ps" | "ps1" | "powershell" | "pwsh" => Self::PowerShell,
            "html" | "xml" | "svg" => Self::Markup,
            "css" | "scss" => Self::Css,
            "sql" | "postgresql" | "sqlite" => Self::Sql,
            _ => Self::Plain,
        }
    }

    fn line_comment(self, s: &str) -> bool {
        match self {
            Self::Rust | Self::JavaScript | Self::Json | Self::Css | Self::CLike => {
                s.starts_with("//")
            }
            Self::Python | Self::Shell | Self::PowerShell | Self::Config => s.starts_with('#'),
            Self::Sql => s.starts_with("--"),
            _ => false,
        }
    }

    fn block_comment(self, s: &str) -> Option<(&'static str, &'static str)> {
        let (start, end) = match self {
            Self::Rust | Self::JavaScript | Self::Json | Self::Css | Self::CLike | Self::Sql => {
                ("/*", "*/")
            }
            Self::Markup => ("<!--", "-->"),
            Self::PowerShell => ("<#", "#>"),
            _ => return None,
        };
        s.starts_with(start).then_some((start, end))
    }

    fn keyword(self, word: &str) -> bool {
        use Language::*;
        match self {
            Rust => matches!(
                word,
                "as" | "async"
                    | "await"
                    | "break"
                    | "const"
                    | "continue"
                    | "crate"
                    | "dyn"
                    | "else"
                    | "enum"
                    | "extern"
                    | "false"
                    | "fn"
                    | "for"
                    | "if"
                    | "impl"
                    | "in"
                    | "let"
                    | "loop"
                    | "match"
                    | "mod"
                    | "move"
                    | "mut"
                    | "pub"
                    | "ref"
                    | "return"
                    | "self"
                    | "Self"
                    | "static"
                    | "struct"
                    | "super"
                    | "trait"
                    | "true"
                    | "type"
                    | "unsafe"
                    | "use"
                    | "where"
                    | "while"
                    | "yield"
            ),
            Python => matches!(
                word,
                "and"
                    | "as"
                    | "assert"
                    | "async"
                    | "await"
                    | "break"
                    | "case"
                    | "class"
                    | "continue"
                    | "def"
                    | "del"
                    | "elif"
                    | "else"
                    | "except"
                    | "False"
                    | "finally"
                    | "for"
                    | "from"
                    | "global"
                    | "if"
                    | "import"
                    | "in"
                    | "is"
                    | "lambda"
                    | "match"
                    | "None"
                    | "nonlocal"
                    | "not"
                    | "or"
                    | "pass"
                    | "raise"
                    | "return"
                    | "True"
                    | "try"
                    | "while"
                    | "with"
                    | "yield"
            ),
            JavaScript => matches!(
                word,
                "abstract"
                    | "as"
                    | "async"
                    | "await"
                    | "break"
                    | "case"
                    | "catch"
                    | "class"
                    | "const"
                    | "continue"
                    | "debugger"
                    | "default"
                    | "delete"
                    | "do"
                    | "else"
                    | "enum"
                    | "export"
                    | "extends"
                    | "false"
                    | "finally"
                    | "for"
                    | "from"
                    | "function"
                    | "if"
                    | "implements"
                    | "import"
                    | "in"
                    | "instanceof"
                    | "interface"
                    | "let"
                    | "new"
                    | "null"
                    | "of"
                    | "private"
                    | "protected"
                    | "public"
                    | "readonly"
                    | "return"
                    | "static"
                    | "super"
                    | "switch"
                    | "this"
                    | "throw"
                    | "true"
                    | "try"
                    | "type"
                    | "typeof"
                    | "undefined"
                    | "var"
                    | "void"
                    | "while"
                    | "with"
                    | "yield"
            ),
            CLike => matches!(
                word,
                "abstract"
                    | "auto"
                    | "bool"
                    | "break"
                    | "case"
                    | "catch"
                    | "char"
                    | "class"
                    | "const"
                    | "continue"
                    | "default"
                    | "defer"
                    | "delete"
                    | "do"
                    | "double"
                    | "else"
                    | "enum"
                    | "extern"
                    | "false"
                    | "final"
                    | "float"
                    | "for"
                    | "func"
                    | "goto"
                    | "if"
                    | "import"
                    | "int"
                    | "interface"
                    | "long"
                    | "namespace"
                    | "new"
                    | "nil"
                    | "null"
                    | "package"
                    | "private"
                    | "protected"
                    | "public"
                    | "range"
                    | "return"
                    | "short"
                    | "signed"
                    | "sizeof"
                    | "static"
                    | "struct"
                    | "switch"
                    | "template"
                    | "this"
                    | "throw"
                    | "true"
                    | "try"
                    | "type"
                    | "typedef"
                    | "union"
                    | "unsigned"
                    | "using"
                    | "var"
                    | "virtual"
                    | "void"
                    | "volatile"
                    | "while"
            ),
            Sql => [
                "select", "from", "where", "order", "group", "by", "having", "limit", "and", "or",
                "not", "null", "true", "false", "as", "inner", "left", "right", "join", "on",
                "insert", "into", "values", "update", "set", "delete", "create", "alter", "drop",
                "table", "distinct", "union", "with", "exists", "in", "like", "is", "asc", "desc",
                "case", "when", "then", "end",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword)),
            Shell | PowerShell => matches!(
                word,
                "if" | "then"
                    | "else"
                    | "elseif"
                    | "elif"
                    | "fi"
                    | "for"
                    | "foreach"
                    | "while"
                    | "do"
                    | "done"
                    | "in"
                    | "case"
                    | "esac"
                    | "function"
                    | "return"
                    | "end"
                    | "exit"
                    | "param"
                    | "switch"
                    | "begin"
                    | "process"
                    | "trap"
                    | "try"
                    | "catch"
                    | "finally"
                    | "export"
                    | "local"
                    | "break"
                    | "continue"
                    | "$true"
                    | "$false"
                    | "$null"
            ),
            Json | Config => matches!(word, "false" | "true" | "null" | "yes" | "no"),
            _ => false,
        }
    }
}

pub fn highlight(code: &str, language: Language) -> Text {
    if language == Language::Plain {
        return Text::new(code);
    }

    let mut output = Text::default();
    let mut plain = 0;
    let mut offset = 0;
    let mut in_tag = false;
    while offset < code.len() {
        let remaining = &code[offset..];
        let (length, token) = next_token(remaining, language, &mut in_tag);
        if let Some(token) = token {
            output.push(&code[plain..offset]);
            let start = output.len();
            output.push(&remaining[..length]);
            output.style(start, Style::Syntax(token));
            plain = offset + length;
        }
        offset += length;
    }
    output.push(&code[plain..]);
    output
}

fn next_token(s: &str, language: Language, in_tag: &mut bool) -> (usize, Option<Token>) {
    use Language::*;
    if language.line_comment(s) {
        return (s.find('\n').unwrap_or(s.len()), Some(Token::Comment));
    }
    if let Some((start, end)) = language.block_comment(s) {
        let count = if language == Rust {
            nested_comment(s)
        } else {
            s[start.len()..]
                .find(end)
                .map_or(s.len(), |n| start.len() + n + end.len())
        };
        return (count, Some(Token::Comment));
    }

    let ch = s.as_bytes()[0];
    if language == Markup {
        match ch {
            b'<' => {
                *in_tag = true;
                return (1, Some(Token::Keyword));
            }
            b'>' if *in_tag => {
                *in_tag = false;
                return (1, Some(Token::Keyword));
            }
            _ if !*in_tag => return (s.find('<').unwrap_or(s.len()), None),
            _ => (),
        }
    }
    if language == Rust
        && let Some(length) = rust_raw_string(s)
    {
        return (length, Some(Token::String));
    }
    if (matches!(ch, b'\'' | b'"') && (language != Rust || ch == b'"' || rust_char(s)))
        || (ch == b'`' && matches!(language, JavaScript | Shell | CLike | Sql))
    {
        let length = quoted(s, ch, matches!(language, Python | Config));
        let token = if language == Json && s[length..].trim_start().starts_with(':') {
            Token::Name
        } else {
            Token::String
        };
        return (length, Some(token));
    }
    if ch.is_ascii_digit() {
        let length = s
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_'))
            .count();
        return (length, Some(Token::Number));
    }

    let first = s.chars().next().unwrap();
    if first.is_alphabetic() || matches!(first, '_' | '$') {
        let length = s
            .char_indices()
            .find(|&(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '$')))
            .map_or(s.len(), |(index, _)| index);
        let word = &s[..length];
        let next = s[length..].trim_start_matches([' ', '\t']);
        let token = if language.keyword(word) || (language == CLike && word == "include") {
            Some(Token::Keyword)
        } else if language == Markup
            || next.starts_with(['(', '!'])
            || (matches!(language, Config | Css) && next.starts_with([':', '=']))
            || (matches!(language, Shell | PowerShell) && first == '$')
        {
            Some(Token::Name)
        } else {
            None
        };
        return (length, token);
    }
    (first.len_utf8(), None)
}

fn rust_raw_string(s: &str) -> Option<usize> {
    let prefix = if s.starts_with("br") || s.starts_with("cr") {
        2
    } else if s.starts_with('r') {
        1
    } else {
        return None;
    };
    let hashes = s[prefix..].bytes().take_while(|byte| *byte == b'#').count();
    let start = prefix + hashes;
    if s.as_bytes().get(start) != Some(&b'"') {
        return None;
    }
    for (offset, _) in s[start + 1..].match_indices('"') {
        let end = start + offset + 2;
        if s.as_bytes()
            .get(end..end + hashes)
            .is_some_and(|bytes| bytes.iter().all(|b| *b == b'#'))
        {
            return Some(end + hashes);
        }
    }
    Some(s.len())
}

fn rust_char(s: &str) -> bool {
    let next = &s[1..];
    next.starts_with('\\')
        || next
            .chars()
            .next()
            .is_some_and(|c| next[c.len_utf8()..].starts_with('\''))
}

fn nested_comment(s: &str) -> usize {
    let mut depth = 1;
    let mut index = 2;
    while let Some(marker) = s.as_bytes().get(index..index + 2) {
        match marker {
            b"/*" => {
                depth += 1;
                index += 2;
            }
            b"*/" => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    s.len()
}

fn quoted(s: &str, quote: u8, triple: bool) -> usize {
    let bytes = s.as_bytes();
    let delimiter = [quote; 3];
    let count = if triple && bytes.starts_with(&delimiter) {
        3
    } else {
        1
    };
    let mut index = count;
    while index < bytes.len() {
        if bytes[index..].starts_with(&delimiter[..count]) {
            return index + count;
        }
        if bytes[index] == b'\\' {
            index += 1;
        }
        index += 1;
    }
    s.len()
}
