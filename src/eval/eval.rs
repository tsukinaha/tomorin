use std::borrow::Cow;

use htmlescape::{encode_attribute, encode_minimal};
use once_cell::sync::Lazy;
use phf::phf_map;
use regex::{Captures, Regex};

use crate::eval::types::{Channel, Response};
use unicode_width::UnicodeWidthChar;

/// Normalize the mistakenly inputted Unicode character to the corresponding ASCII character.
///
/// For the table what characters this function will convert, you can refer to
/// [`UNICODE_CHARS_MAP`].
///
/// Time complexity of this is `O(n)`.
pub fn normalize_unicode_chars(input: &str) -> Cow<'_, str> {
    // If the input is ASCII, there is no need to normalize.
    if input.is_ascii() {
        return input.into();
    }

    let mut output = String::with_capacity(input.len());

    for c in input.chars() {
        if let Some(replacement) = UNICODE_CHARS_MAP.get(&c) {
            output.push_str(replacement);
        } else {
            output.push(c);
        }
    }

    output.into()
}

const PRELUDE: &str = include_str!("prelude.res.rs");

pub fn generate_code_to_send(code: &str) -> String {
    if code.contains("fn main()") {
        return code.to_string();
    }
    macro_rules! template {
        ($($line:expr,)+) => {
            concat!($($line, '\n',)+)
        }
    }
    let (header, body) = extract_code_headers(code);
    tracing::debug!("extract: {:?} -> ({:?}, {:?})", code, header, body);
    let code = if body.contains("println!") || body.contains("print!") {
        format!("{{\n{code}\n}};")
    } else {
        format!(
            template! {
                // Template below would provide the indent of this line.
                "println!(\"{{:?}}\", {{",
                "        {code}",
                "    }});",
            },
            code = body
        )
    };
    format!(
        template! {
            "#![allow(warnings)]",
            "{header}",
            "{prelude}",
            "fn main() -> Result<(), Box<dyn std::error::Error>> {{",
            "    {code}",
            "    Ok(())",
            "}}",
        },
        header = header,
        prelude = PRELUDE,
        code = code,
    )
}

static UNICODE_CHARS_MAP: phf::Map<char, &str> = phf_map! {
    '“' => "\"",
    '”' => "\"",
    '‘' => "\'",
    '’' => "\'",
    '—' => "--",
    '\u{a0}' => " ",
};

fn extract_code_headers(code: &str) -> (&str, &str) {
    use combine::parser::char::{alpha_num, space, spaces, string};
    use combine::parser::choice::choice;
    use combine::parser::combinator::{attempt, ignore};
    use combine::parser::range::recognize;
    use combine::parser::repeat::{skip_many, skip_many1};
    use combine::parser::token::{none_of, token};
    use combine::parser::Parser;
    use std::iter::once;
    let spaces1 = || (space(), spaces());
    let attr_content = || (token('['), skip_many(none_of(once(']'))), token(']'));
    let outer_attr = (token('#'), spaces(), attr_content());
    let inner_attr = (token('#'), spaces(), token('!'), spaces(), attr_content());
    let extern_crate = (
        skip_many(outer_attr),
        spaces(),
        string("extern"),
        spaces1(),
        string("crate"),
        spaces1(),
        skip_many1(choice((alpha_num(), token('_')))),
        spaces(),
        token(';'),
    );
    let mut header = recognize((
        spaces(),
        skip_many((
            choice((attempt(ignore(extern_crate)), attempt(ignore(inner_attr)))),
            spaces(),
        )),
    ));
    header.parse(code).unwrap_or_else(|_| {
        debug_assert!(false, "extract_code_headers should always succeed");
        tracing::warn!("failed to split code: {}", code);
        ("", code)
    })
}

pub fn truncate_output(output: &str, max_lines: usize, max_total_columns: usize) -> Cow<'_, str> {
    let mut line_count = 0;
    let mut column_count = 0;
    for (pos, c) in output.char_indices() {
        column_count += c.width_cjk().unwrap_or(1);
        if column_count > max_total_columns {
            let mut truncate_width = 0;
            for (pos, c) in output[..pos].char_indices().rev() {
                truncate_width += c.width_cjk().unwrap_or(1);
                if truncate_width >= 3 {
                    return format!("{}...", &output[..pos]).into();
                }
            }
        }
        if c == '\n' {
            line_count += 1;
            if line_count == max_lines {
                return format!("{}...", &output[..pos]).into();
            }
        }
    }
    output.into()
}

pub fn generate_result_from_response(resp: Response, channel: Channel, is_private: bool) -> String {
    if resp.success {
        let output = resp.stdout.trim();
        let output = if is_private {
            output.into()
        } else {
            const MAX_LINES: usize = 3;
            const MAX_TOTAL_COLUMNS: usize = MAX_LINES * 72;
            truncate_output(output, MAX_LINES, MAX_TOTAL_COLUMNS)
        };
        if output.is_empty() {
            return "(no output)".to_string();
        }
        return encode_minimal(&output);
    }

    static RE_ERROR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^error\[(E\d{4})\]:").unwrap());
    static RE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`(.+?)`").unwrap());
    static RE_ISSUE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\(see issue #(\d+)\)").unwrap());
    let mut return_line: Option<&str> = None;
    for line in resp.stderr.split('\n') {
        let line = line.trim();
        if line.starts_with("Compiling")
            || line.starts_with("Finished")
            || line.starts_with("Running")
            || line.is_empty()
        {
            continue;
        }
        if line.starts_with("error") {
            return_line = Some(line);
            break;
        }
        if return_line.is_none() {
            return_line = Some(line);
        }
    }
    if let Some(line) = return_line {
        let line = encode_minimal(line);
        let line = RE_ERROR.replacen(&line, 1, |captures: &Captures<'_>| {
            let err_num = captures.get(1).unwrap().as_str();
            let url = format!(
                "https://doc.rust-lang.org/{}/error-index.html#{}",
                channel.as_str(),
                err_num,
            );
            format!(
                r#"error<a href="{}">[{}]</a>:"#,
                encode_attribute(&url),
                err_num,
            )
        });
        let line = RE_CODE.replace_all(&line, |captures: &Captures<'_>| {
            format!("<code>{}</code>", captures.get(1).unwrap().as_str())
        });
        let line = RE_ISSUE.replacen(&line, 1, |captures: &Captures<'_>| {
            let issue_num = captures.get(1).unwrap().as_str();
            let url = format!("https://github.com/rust-lang/rust/issues/{issue_num}");
            format!(r#"(see issue <a href="{url}">#{issue_num}</a>)"#)
        });
        format!("{line}")
    } else {
        "(nothing??)".to_string()
    }
}