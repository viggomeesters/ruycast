//! Pure Raycast mapping. Filesystem discovery and CAS publication belong to the
//! caller; this module never executes a command, writes a file or logs input.
use objc2_foundation::NSString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, String>;
#[derive(Default)]
pub struct Options {
    pub include_templates: bool,
    pub include_snippets: bool,
}
pub struct Application {
    pub path: PathBuf,
    pub metadata: Value,
    pub non_json_title: bool,
}
pub struct Prepared {
    pub commands: Vec<Value>,
    pub counts: BTreeMap<String, u64>,
    pub non_json_title: bool,
}
impl Prepared {
    pub fn summary(&self, applied: bool) -> Value {
        json!({"applied":applied,"imported":self.counts,"totalCommands":self.commands.len(),"hotkeysImported":0})
    }
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Expected string {key}."))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Expected array {key}."))
}
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(n) => n.as_f64() != Some(0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
fn lower(s: &str) -> String {
    NSString::from_str(s).lowercaseString().to_string()
}
fn strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}
fn aliases(key: &str, roots: &[Value], names: &mut BTreeSet<String>) -> Result<Vec<String>> {
    let Some(row) = roots
        .iter()
        .find(|row| row.get("key").and_then(Value::as_str) == Some(key))
    else {
        return Ok(vec![]);
    };
    let alias = strip(
        row.get("alias")
            .map_or(Ok(""), |v| v.as_str().ok_or("Expected string alias."))?,
    );
    Ok(if !alias.is_empty() && names.insert(lower(alias)) {
        vec![alias.into()]
    } else {
        vec![]
    })
}
fn unique_rows(rows: &[Value]) -> Result<Vec<Value>> {
    let mut indexes = BTreeMap::new();
    let mut result = vec![];
    for row in rows {
        let key = string(row, "key")?;
        if let Some(index) = indexes.get(key) {
            result[*index] = row.clone()
        } else {
            indexes.insert(key.to_owned(), result.len());
            result.push(row.clone())
        }
    }
    Ok(result)
}
fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(a), Some(z)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            ) {
                out.push((a * 16 + z) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn url_parts(url: &str) -> Result<(String, String, String)> {
    let cleaned = url
        .trim_start_matches(|c: char| (c as u32) <= 0x20)
        .replace(['\t', '\r', '\n'], "");
    let mut rest = cleaned.as_str();
    let mut scheme = String::new();
    if let Some((head, tail)) = rest.split_once(':') {
        if head.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
            && head
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
        {
            scheme = head.to_ascii_lowercase();
            rest = tail
        }
    }
    let mut netloc = String::new();
    if let Some(tail) = rest.strip_prefix("//") {
        let end = tail.find(['/', '?', '#']).unwrap_or(tail.len());
        netloc = tail[..end].into();
        rest = &tail[end..];
        let host = netloc.rsplit('@').next().unwrap();
        if host.contains(['[', ']']) {
            let bracket = host
                .strip_prefix('[')
                .and_then(|s| s.split_once(']'))
                .ok_or("Invalid IPv6 URL.")?;
            if !bracket.1.is_empty() && !bracket.1.starts_with(':')
                || bracket.1.contains(['[', ']'])
            {
                return Err("Invalid IPv6 URL.".into());
            }
            let h = bracket.0;
            let future = h
                .strip_prefix('v')
                .and_then(|v| v.split_once('.'))
                .is_some_and(|(v, t)| {
                    !v.is_empty() && v.bytes().all(|b| b.is_ascii_hexdigit()) && !t.is_empty()
                });
            let (address, zone) = h.split_once('%').map_or((h, None), |(a, z)| (a, Some(z)));
            if !future
                && (address.parse::<std::net::Ipv6Addr>().is_err()
                    || zone.is_some_and(|z| z.is_empty() || z.contains('%')))
            {
                return Err("Invalid IPv6 URL.".into());
            }
        }
        let check = netloc.replace(['@', ':', '#', '?'], "");
        let normalized = NSString::from_str(&check)
            .precomposedStringWithCompatibilityMapping()
            .to_string();
        if normalized != check && normalized.contains(['/', '?', '#', '@', ':']) {
            return Err("URL netloc contains invalid characters under NFKC normalization.".into());
        }
    }
    Ok((
        scheme,
        netloc,
        rest.split(['?', '#']).next().unwrap().into(),
    ))
}
fn target(url: &str) -> Result<Option<(&'static str, String)>> {
    let (scheme, netloc, path) = url_parts(url)?;
    if !scheme.is_empty() && !["javascript", "data", "file"].contains(&scheme.as_str()) {
        return Ok(Some(("url", url.into())));
    }
    if scheme == "file" && ["", "localhost"].contains(&netloc.as_str()) {
        return Ok(Some(("path", unquote(&path))));
    }
    if url.starts_with('/') || url.starts_with("~/") {
        return Ok(Some(("path", url.into())));
    }
    Ok(None)
}
pub fn prepare(
    config: &Value,
    data: &Value,
    apps: &BTreeMap<String, Application>,
    options: &Options,
) -> Result<Prepared> {
    let roots = unique_rows(array(
        data.get("builtin_package_rootSearch")
            .ok_or("Missing rootSearch package.")?,
        "rootSearch",
    )?)?;
    // Preserve validation of the legacy activity map even though mapping uses no activity.
    unique_rows(array(
        data.get("raycast_user_activity")
            .ok_or("Missing activity package.")?,
        "activityInfo",
    )?)?;
    let mut commands = array(config, "commands")?.clone();
    let mut names = BTreeSet::new();
    let mut by_app = BTreeMap::new();
    for (index, command) in commands.iter().enumerate() {
        names.insert(lower(string(command, "id")?));
        if let Some(aliases) = command.get("aliases") {
            for alias in aliases.as_array().ok_or("Expected aliases array.")? {
                names.insert(lower(alias.as_str().ok_or("Expected string alias.")?));
            }
        }
        if string(command, "kind")? == "app" {
            by_app.insert(string(command, "target")?.to_owned(), index);
        }
    }
    let mut non_json_title = false;
    let mut counts = [
        "appAliases",
        "apps",
        "quicklinks",
        "parameterizedSkipped",
        "disabledSkipped",
        "unsupportedSkipped",
    ]
    .map(|s| (s.to_owned(), 0))
    .into_iter()
    .collect::<BTreeMap<_, u64>>();
    for row in &roots {
        if row.get("type").and_then(Value::as_str) != Some("systemApp")
            || !row.get("alias").is_some_and(truth)
        {
            continue;
        }
        let bid = string(row, "key")?;
        let Some(app) = apps.get(bid) else {
            *counts.get_mut("unsupportedSkipped").unwrap() += 1;
            continue;
        };
        let extra = aliases(bid, &roots, &mut names)?;
        if extra.is_empty() {
            continue;
        }
        if let Some(index) = by_app.get(bid) {
            let object = commands[*index]
                .as_object_mut()
                .ok_or("Expected command object.")?;
            object
                .entry("aliases")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or("Expected aliases array.")?
                .extend(extra.iter().map(|s| json!(s)));
        } else {
            let id = format!("app.{bid}");
            if names.contains(&lower(&id)) {
                continue;
            }
            let stem = app.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let title = app
                .metadata
                .get("CFBundleDisplayName")
                .or_else(|| app.metadata.get("CFBundleName"))
                .cloned()
                .unwrap_or_else(|| json!(stem));
            let index = commands.len();
            non_json_title |= app.non_json_title;
            commands.push(json!({"id":id,"title":title,"kind":"app","target":bid,"aliases":extra}));
            names.insert(lower(&id));
            by_app.insert(bid.to_owned(), index);
            *counts.get_mut("apps").unwrap() += 1;
        }
        *counts.get_mut("appAliases").unwrap() += extra.len() as u64;
    }
    for link in array(
        data.get("builtin_package_quicklinks")
            .ok_or("Missing quicklinks package.")?,
        "quicklinks",
    )? {
        if link.get("isEnabled").is_some_and(|v| !truth(v)) {
            *counts.get_mut("disabledSkipped").unwrap() += 1;
            continue;
        }
        let url = string(link, "url")?;
        if !options.include_templates && url.contains(['{', '}']) {
            *counts.get_mut("parameterizedSkipped").unwrap() += 1;
            continue;
        }
        let uuid = string(link, "uuid")?;
        let id = format!("link.{}", lower(uuid));
        if names.contains(&lower(&id)) {
            continue;
        }
        let Some((kind, target)) = target(url)? else {
            *counts.get_mut("unsupportedSkipped").unwrap() += 1;
            continue;
        };
        names.insert(lower(&id));
        let extra = aliases(&format!("raycastQuicklink_{uuid}"), &roots, &mut names)?;
        commands.push(json!({"id":id,"title":link.get("name").ok_or("Missing quicklink name.")?,"kind":kind,"target":target,"aliases":extra}));
        *counts.get_mut("quicklinks").unwrap() += 1;
    }
    if options.include_snippets {
        counts.insert("snippets".into(), 0);
        let mut keywords = commands
            .iter()
            .filter_map(|c| c.get("keyword").and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let snippets: &[Value] = match data.get("builtin_package_snippets") {
            None => &[],
            Some(Value::Object(package)) => match package.get("snippets") {
                None => &[],
                Some(value) => value.as_array().ok_or("Expected snippets array.")?,
            },
            Some(_) => return Err("Expected snippets package object.".into()),
        };
        for snippet in snippets {
            let name = string(snippet, "name")?;
            let keyword = strip(snippet.get("alias").map_or(Ok(""), |v| {
                v.as_str().ok_or("Expected string snippet alias.")
            })?);
            let id = format!(
                "snippet.{}",
                &format!(
                    "{:x}",
                    Sha256::digest(format!("{name}\0{keyword}").as_bytes())
                )[..20]
            );
            if names.contains(&lower(&id)) {
                continue;
            }
            let mut command = json!({"id":id,"title":name,"kind":"snippet","target":snippet.get("text").ok_or("Missing snippet text.")?});
            if !keyword.is_empty() && keywords.insert(keyword.into()) {
                command["keyword"] = json!(keyword)
            }
            commands.push(command);
            names.insert(lower(&id));
            *counts.get_mut("snippets").unwrap() += 1;
        }
    }
    Ok(Prepared {
        commands,
        counts,
        non_json_title,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn data() -> Value {
        json!({"builtin_package_rootSearch":{"rootSearch":[]},"raycast_user_activity":{"activityInfo":[]},"builtin_package_quicklinks":{"quicklinks":[]}})
    }
    #[test]
    fn quicklinks_keep_order_counters_aliases_templates_and_file_decoding() {
        let mut d = data();
        d["builtin_package_rootSearch"]["rootSearch"] = json!([{"key":"raycastQuicklink_A","alias":" First "},{"key":"raycastQuicklink_A","alias":" Final "}]);
        d["builtin_package_quicklinks"]["quicklinks"] = json!([
            {"uuid":"A","name":"Link","url":"https://example.com"},
            {"uuid":"a","name":"Duplicate","url":"https://other.example"},
            {"uuid":"B","name":"File","url":"file://localhost/tmp/a%20b%FF?ignored#fragment"},
            {"uuid":"C","name":"Template","url":"https://example.com/{query}"},
            {"isEnabled":false}, {"uuid":"D","name":"Unsafe","url":"javascript:alert(1)"}]);
        let result = prepare(
            &json!({"commands":[]}),
            &d,
            &BTreeMap::new(),
            &Options::default(),
        )
        .unwrap();
        assert_eq!(result.counts["quicklinks"], 2);
        assert_eq!(result.counts["disabledSkipped"], 1);
        assert_eq!(result.counts["parameterizedSkipped"], 1);
        assert_eq!(result.counts["unsupportedSkipped"], 1);
        assert_eq!(result.commands[0]["aliases"], json!(["Final"]));
        assert_eq!(result.commands[1]["target"], "/tmp/a b�");
        assert!(result.summary(false)["imported"].get("snippets").is_none());
        let included = prepare(
            &json!({"commands":[]}),
            &d,
            &BTreeMap::new(),
            &Options {
                include_templates: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(included.counts["quicklinks"], 3);
    }
    #[test]
    fn app_aliases_and_snippets_preserve_collisions_ids_and_private_inputs() {
        let config = json!({"commands":[{"id":"existing","kind":"app","target":"org.app","aliases":["used"],"keyword":"Key"}]});
        let mut d = data();
        d["builtin_package_rootSearch"]["rootSearch"] = json!([{"key":"org.app","type":"systemApp","alias":" New "},{"key":"org.new","type":"systemApp","alias":"used"},{"key":"org.missing","type":"systemApp","alias":"Absent"}]);
        d["builtin_package_snippets"] = json!({"snippets":[{"name":"Private Ω","alias":" Key ","text":"private snippet"},{"name":"Private Ω","alias":"Key","text":"duplicate"},{"name":"Other","alias":"key","text":"other"}]});
        let apps = [
            (
                "org.app".into(),
                Application {
                    path: "/owned/App.app".into(),
                    non_json_title: false,
                    metadata: json!({}),
                },
            ),
            (
                "org.new".into(),
                Application {
                    path: "/owned/New.app".into(),
                    non_json_title: false,
                    metadata: json!({"CFBundleDisplayName":"New"}),
                },
            ),
        ]
        .into_iter()
        .collect();
        let config_before = config.clone();
        let data_before = d.clone();
        let result = prepare(
            &config,
            &d,
            &apps,
            &Options {
                include_snippets: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(config, config_before);
        assert_eq!(d, data_before);
        assert_eq!(result.commands[0]["aliases"], json!(["used", "New"]));
        assert_eq!(result.counts["apps"], 0);
        assert_eq!(result.counts["appAliases"], 1);
        assert_eq!(result.counts["unsupportedSkipped"], 1);
        assert_eq!(result.counts["snippets"], 2);
        assert!(result.commands[1].get("keyword").is_none());
        assert_eq!(result.commands[2]["keyword"], "key");
        assert_eq!(result.commands[1]["id"], "snippet.6911f486c74653fd4570");
        assert_eq!(result.commands[1]["target"], "private snippet");
    }
    #[test]
    fn missing_and_empty_snippet_packages_default_to_empty_but_wrong_types_refuse() {
        let options = Options {
            include_snippets: true,
            ..Options::default()
        };
        for package in [None, Some(json!({})), Some(json!({"snippets":[]}))] {
            let mut d = data();
            if let Some(package) = package {
                d["builtin_package_snippets"] = package;
            }
            let result = prepare(&json!({"commands":[]}), &d, &BTreeMap::new(), &options).unwrap();
            assert_eq!(result.counts["snippets"], 0);
            assert!(result.commands.is_empty());
        }
        for package in [
            Value::Null,
            json!({"snippets":null}),
            json!({"snippets":{}}),
        ] {
            let mut d = data();
            d["builtin_package_snippets"] = package;
            assert!(prepare(&json!({"commands":[]}), &d, &BTreeMap::new(), &options).is_err());
        }
    }
    #[test]
    fn new_apps_keep_title_fallback_and_reserve_alias_before_id_collision() {
        for (metadata, title) in [
            (
                json!({"CFBundleDisplayName":"Display","CFBundleName":"Name"}),
                "Display",
            ),
            (json!({"CFBundleName":"Name"}), "Name"),
            (json!({}), "Fallback"),
        ] {
            let mut d = data();
            d["builtin_package_rootSearch"]["rootSearch"] =
                json!([{"key":"org.new","type":"systemApp","alias":"Fresh"}]);
            let apps = [(
                "org.new".into(),
                Application {
                    path: "/owned/Fallback.app".into(),
                    non_json_title: false,
                    metadata,
                },
            )]
            .into_iter()
            .collect();
            let result = prepare(&json!({"commands":[]}), &d, &apps, &Options::default()).unwrap();
            assert_eq!(
                result.commands[0],
                json!({"id":"app.org.new","title":title,"kind":"app","target":"org.new","aliases":["Fresh"]})
            );
            assert_eq!(result.counts["apps"], 1);
            assert_eq!(result.counts["appAliases"], 1);
        }
        let mut d = data();
        d["builtin_package_rootSearch"]["rootSearch"] = json!([{"key":"org.new","type":"systemApp","alias":"Reserve"},{"key":"org.other","type":"systemApp","alias":"Reserve"}]);
        let apps = ["org.new", "org.other"]
            .map(|id| {
                (
                    id.into(),
                    Application {
                        path: "/owned/App.app".into(),
                        non_json_title: false,
                        metadata: json!({}),
                    },
                )
            })
            .into_iter()
            .collect();
        let original =
            json!({"commands":[{"id":"APP.org.new","kind":"url","target":"https://example.com"}]});
        let result = prepare(&original, &d, &apps, &Options::default()).unwrap();
        assert_eq!(
            result.commands,
            original["commands"].as_array().unwrap().clone()
        );
        assert_eq!(result.counts["apps"], 0);
        assert_eq!(result.counts["appAliases"], 0);
    }
    #[test]
    fn invalid_input_refuses_and_url_parsing_keeps_legacy_classification() {
        assert_eq!(
            target("FILE:///tmp/a%20b").unwrap(),
            Some(("path", "/tmp/a b".into()))
        );
        assert!(target("file://REMOTE/tmp/a").unwrap().is_none());
        assert_eq!(
            target("~/private?a#b").unwrap(),
            Some(("path", "~/private?a#b".into()))
        );
        assert!(target("https://[broken]/").is_err());
        assert!(target("https://example.com／evil").is_err());
        assert_eq!(
            target(" https://example.com").unwrap(),
            Some(("url", " https://example.com".into()))
        );
        assert!(prepare(
            &json!({"commands":[]}),
            &json!({}),
            &BTreeMap::new(),
            &Options::default()
        )
        .is_err());
        assert_eq!(lower("ΟΣ"), "ος");
        assert_eq!(strip("\u{1c} alias \u{1f}"), "alias");
    }
}
