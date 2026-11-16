//! Read-only installed toolchain discovery; driver argument text is data, never executed.
use super::*;
use serde_json::{json, Value};
use std::time::Duration;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    deadline_nanos: u64,
}
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
fn capture(
    root: &Path,
    lease: &BuildLock,
    program: &str,
    args: &[&str],
    deadline: u64,
    seconds: u64,
    limit: usize,
) -> Result<(String, String)> {
    let mut command = Command::new(program);
    command.args(args).current_dir(root);
    lease.apply_to_child(&mut command)?;
    lease.validate()?;
    let budget = parity_bootstrap::inner_budget(deadline)?.min(Duration::from_secs(seconds));
    let result = process::capture(command, budget, limit)?;
    lease.validate()?;
    let stdout = text(result.stdout)?;
    let stderr = text(result.stderr)?;
    if !result.status.success() {
        return Err(format!("{program} failed ({}): {stderr}", result.status));
    }
    Ok((stdout, stderr))
}
fn trimmed(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}'))
}
pub(super) fn run(root: &Path, lease: &BuildLock, stage: &str, payload: &str) -> Result<Value> {
    let request: Request = serde_json::from_str(payload).map_err(error)?;
    match stage {
        "discover" => {
            let (sdk, sdk_stderr) = capture(
                root,
                lease,
                "xcrun",
                &["--show-sdk-path"],
                request.deadline_nanos,
                10,
                1024 * 1024,
            )?;
            eprint!("{sdk_stderr}");
            let (stdout, stderr) = capture(
                root,
                lease,
                "swiftc",
                &[
                    "-###",
                    "-typecheck",
                    "Sources/RuycastApp/FeatureState.swift",
                ],
                request.deadline_nanos,
                15,
                8 * 1024 * 1024,
            )?;
            Ok(json!({"sdk":trimmed(&sdk),"plugins":plugins(&(stdout+&stderr))?}))
        }
        "version" => {
            let (version, stderr) = capture(
                root,
                lease,
                "swiftc",
                &["--version"],
                request.deadline_nanos,
                10,
                1024 * 1024,
            )?;
            eprint!("{stderr}");
            Ok(json!({"compilerVersion":trimmed(&version)}))
        }
        _ => Err("Unknown toolchain discovery stage.".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
