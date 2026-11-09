use crate::system_search::Ticket;
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSCharacterSet, NSString};
use rustcast_core::Command;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;
pub fn query(value: &str) -> Result<String, String> {
    autoreleasepool(|_| {
        let text = NSString::from_str(value)
            .stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet())
            .to_string();
        let text = text.graphemes(true).take(512).collect::<String>();
        if NSString::from_str(&text)
            .rangeOfCharacterFromSet(&NSCharacterSet::controlCharacterSet())
            .length
            > 0
        {
            return Err("Enter a file name without control characters.".into());
        }
        Ok(text)
    })
}
pub fn predicate(query: &str) -> String {
    format!(
        "kMDItemFSName == \"*{}*\"cd",
        query
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('*', "\\*")
            .replace('?', "\\?")
    )
}
#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
    paths: Vec<String>,
    seen: HashSet<String>,
}
impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|b| *b == 0) {
            let value = String::from_utf8_lossy(&self.pending[..end]).into_owned();
            self.pending.drain(..=end);
            if value.starts_with('/') && self.seen.insert(value.clone()) {
                self.paths.push(value);
            }
            if self.paths.len() >= 100 {
                return true;
            }
        }
        false
    }
    pub fn commands(self) -> Vec<Command> {
        self.paths.into_iter().map(|path|{
        let title=autoreleasepool(|_|NSString::from_str(&path).lastPathComponent().to_string());
        serde_json::from_value(serde_json::json!({"id":format!("system.file.{:x}",Sha256::digest(path.as_bytes())),"title":title,"kind":"path","target":path,"aliases":[path]})).expect("fixed file command")
    }).collect()
    }
}
pub fn search(value: &str, ticket: &Ticket) -> Result<Vec<Command>, String> {
    let value = query(value)?;
    if value.is_empty() {
        return Ok(vec![]);
    }
    let predicate = predicate(&value);
    collect("/usr/bin/mdfind", &["-0", &predicate], ticket)
}
fn collect(executable: &str, args: &[&str], ticket: &Ticket) -> Result<Vec<Command>, String> {
    let mut parser = Parser::default();
    crate::os_command::file_query(
        executable,
        args,
        ticket,
        |bytes| parser.feed(bytes),
    )
    .map_err(|_| "Could not search files. Try again.")?;
    ticket.check()?;
    Ok(parser.commands())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_predicate_and_normalization() {
        assert_eq!(
            predicate("a\\b\"c*?"),
            "kMDItemFSName == \"*a\\\\b\\\"c\\*\\?*\"cd"
        );
        assert_eq!(query("  file\n").unwrap(), "file");
        assert!(query("a\nb").is_err());
        assert_eq!(query(&"é".repeat(513)).unwrap(), "é".repeat(512));
        assert!(query("👩‍👩‍👧‍👦").is_err()); // Native control set includes ZWJ, as in legacy.
    }
    #[test]
    fn nul_stream_deduplicates_repairs_utf8_and_stops_at100() {
        let mut parser = Parser::default();
        assert!(!parser.feed(b"relative\0/own/a"));
        assert!(!parser.feed(b"\0/own/a\0/own/\xff\0unfinished"));
        let rows = parser.commands();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target, "/own/a");
        assert_eq!(rows[1].target, "/own/�");
        let mut parser = Parser::default();
        let mut bytes = vec![];
        for n in 0..101 {
            bytes.extend_from_slice(format!("/owned/{n}\0").as_bytes());
        }
        assert!(parser.feed(&bytes));
        assert_eq!(parser.commands().len(), 100);
    }
    #[test]
    fn owned_nonzero_collection_keeps_complete_paths() {
        let binary = std::env::current_exe().unwrap();
        let ticket = Ticket {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(8),
            live: std::sync::Arc::new(|| true),
        };
        let rows = collect(binary.to_str().unwrap(), &[
            "--exact", "os_command::tests::owned_file_query_fixture", "--ignored",
            "--nocapture", "--test-threads=1", "--skip", "owned-file-nonzero",
        ], &ticket).expect("Nonzero child status must retain complete file records.");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target, "/owned/first");
        assert_eq!(rows[1].target, "/owned/second");
    }
    #[test]
    fn owned_private_collection_stop_keeps_complete_paths_before_outer_deadline() {
        let binary = std::env::current_exe().unwrap();
        let start = std::time::Instant::now();
        let ticket = Ticket {
            deadline: start + std::time::Duration::from_millis(3600),
            live: std::sync::Arc::new(|| true),
        };
        let rows = collect(binary.to_str().unwrap(), &[
            "--exact", "os_command::tests::owned_file_query_fixture", "--ignored",
            "--nocapture", "--test-threads=1", "--skip", "owned-file-hold",
        ], &ticket).expect("Private collection stop must retain complete file records.");
        assert!(start.elapsed() >= std::time::Duration::from_secs(3));
        assert!(start.elapsed() < std::time::Duration::from_millis(3600));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target, "/owned/first");
        assert_eq!(rows[1].target, "/owned/second");
    }
}
