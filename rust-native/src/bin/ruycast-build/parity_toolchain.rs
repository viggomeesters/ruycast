//! Read-only installed toolchain discovery; driver argument text is data, never executed.
use super::*;
// Python shlex.split(posix=True,comments=False), including quote concatenation,
// empty quoted words, escaped newlines and literal # characters.
fn split(text: &str) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut token = String::new();
    let mut started = false;
    let mut quote = None;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            if quote == Some('"') && ch != '"' && ch != '\\' {
                token.push('\\');
            }
            token.push(ch);
            escaped = false;
            started = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            } else if ch == '\\' && delimiter == '"' {
                escaped = true;
            } else {
                token.push(ch);
            }
        } else {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    if started {
                        result.push(std::mem::take(&mut token));
                        started = false;
                    }
                }
                '\'' | '"' => {
                    quote = Some(ch);
                    started = true;
                }
                '\\' => {
                    escaped = true;
                    started = true;
                }
                _ => {
                    token.push(ch);
                    started = true;
                }
            }
        }
    }
    if escaped {
        return Err("No escaped character".into());
    }
    if quote.is_some() {
        return Err("No closing quotation".into());
    }
    if started {
        result.push(token);
    }
    Ok(result)
}
fn plugins(text: &str) -> Result<Vec<String>> {
    let arguments = split(text)?;
    let mut plugins = Vec::new();
    for (index, arg) in arguments.iter().enumerate() {
        if [
            "-plugin-path",
            "-external-plugin-path",
            "-load-plugin-library",
            "-load-plugin-executable",
        ]
        .contains(&arg.as_str())
        {
            plugins.extend(
                arguments[index..arguments.len().min(index + 2)]
                    .iter()
                    .cloned(),
            );
        }
    }
    Ok(plugins)
}
fn text(bytes: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(bytes)
        .map_err(error)?
        .replace("\r\n", "\n")
        .replace('\r', "\n"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn subprocess_text_normalizes_newlines_and_refuses_bad_utf8() {
        assert_eq!(text(b"a\r\nb\rc\n".to_vec()).unwrap(), "a\nb\nc\n");
        assert!(text(vec![255]).is_err());
    }
    #[test]
    fn quoted_driver_arguments_keep_python_semantics() {
        assert_eq!(
            split("a'b c'\"d\" \"\" x#y").unwrap(),
            vec!["ab cd", "", "x#y"]
        );
        assert_eq!(
            split("\"a\\zb\\\"c\\\\d\" q\\\nr").unwrap(),
            vec!["a\\zb\"c\\d", "q\nr"]
        );
        assert_eq!(split("a\u{b}b\u{c}c").unwrap(), vec!["a\u{b}b\u{c}c"]);
        assert_eq!(split("\\").unwrap_err(), "No escaped character");
        assert_eq!(split("'").unwrap_err(), "No closing quotation");
        assert_eq!(split("\"\\").unwrap_err(), "No escaped character");
    }
    #[test]
    fn all_plugin_flags_preserve_order_and_missing_argument_behavior() {
        assert_eq!(plugins("-plugin-path 'space path' -external-plugin-path a#b -load-plugin-library lib -load-plugin-executable exe").unwrap(),vec!["-plugin-path","space path","-external-plugin-path","a#b","-load-plugin-library","lib","-load-plugin-executable","exe"]);
        assert_eq!(plugins("-plugin-path").unwrap(), vec!["-plugin-path"]);
    }
    #[test]
    #[ignore = "Owned lexer data differential entry, no external program text is executed"]
    fn lexer_oracle_entry() {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).unwrap();
        let values: Vec<String> = serde_json::from_str(&text).unwrap();
        let rows = values
            .iter()
            .map(|value| match split(value) {
                Ok(tokens) => json!({"tokens":tokens}),
                Err(error) => json!({"error":error}),
            })
            .collect::<Vec<_>>();
        println!("LEXER_RESULTS={}", json!(rows));
    }
}
