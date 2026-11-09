//! Immutable enabled catalog and prepared ranking; no filesystem/UI access.
use rustcast_core::{Command, SearchIndex};
use std::collections::HashMap;

#[cfg(test)]
std::thread_local! {
    static WORK: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
}
#[cfg(test)]
pub(crate) fn preparation_work() -> (usize, usize, usize) {
    WORK.with(std::cell::Cell::get)
}

fn same_command(a: &Command, b: &Command) -> bool {
    // Exhaustive typed projection: metadata used by execution is part of the
    // snapshot too. Adding a Command field must update this comparison.
    let Command { id, title, kind, target, aliases, arguments, hotkey, timeout,
        input_prompt, keyword, application_id, enabled, snippet_output } = a;
    let Command { id: bid, title: btitle, kind: bkind, target: btarget,
        aliases: baliases, arguments: barguments, hotkey: bhotkey, timeout: btimeout,
        input_prompt: binput_prompt, keyword: bkeyword, application_id: bapplication_id,
        enabled: benabled, snippet_output: bsnippet_output } = b;
    id == bid && title == btitle && kind == bkind && target == btarget
        && aliases == baliases && arguments == barguments && hotkey == bhotkey
        && timeout == btimeout && input_prompt == binput_prompt && keyword == bkeyword
        && application_id == bapplication_id && enabled == benabled
        && snippet_output == bsnippet_output
}

pub struct Catalog {
    commands: Vec<Command>,
    include_disabled: bool,
    index: SearchIndex,
    weights: Vec<f64>,
    recency: HashMap<String, f64>,
    usage: HashMap<String, i64>,
}

impl Catalog {
    pub fn new(commands: Vec<Command>) -> Self {
        Self::with_disabled(commands, false)
    }
    pub fn with_disabled(mut commands: Vec<Command>, include_disabled: bool) -> Self {
        if !include_disabled {
            commands.retain(|command| command.enabled != Some(false));
        }
        let index = SearchIndex::new(&commands);
        #[cfg(test)]
        WORK.with(|work| { let (index, recency, search) = work.get(); work.set((index + 1, recency, search)); });
        Self {
            commands,
            include_disabled,
            index,
            weights: vec![],
            recency: HashMap::new(),
            usage: HashMap::new(),
        }
    }
    pub fn command(&self, index: usize) -> &Command {
        &self.commands[index]
    }
    pub fn len(&self) -> usize {
        self.commands.len()
    }
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
    pub fn set_usage(&mut self, usage: HashMap<String, i64>) {
        self.usage = usage;
    }
    pub fn prepare_recency(&mut self, entries: &HashMap<String, f64>) {
        #[cfg(test)]
        WORK.with(|work| { let (index, recency, search) = work.get(); work.set((index, recency + 1, search)); });
        self.recency = entries.clone();
        self.weights = if entries.is_empty() {
            vec![]
        } else {
            self.commands
                .iter()
                .map(|command| {
                    entries
                        .get(&command.id)
                        .copied()
                        .filter(|weight| weight.is_finite() && *weight >= 0.)
                        .unwrap_or(0.)
                })
                .collect()
        };
    }
    pub fn replace(&mut self, mut commands: Vec<Command>) -> bool {
        if !self.include_disabled {
            commands.retain(|command| command.enabled != Some(false));
        }
        if self.commands.len() == commands.len()
            && self.commands.iter().zip(&commands).all(|(a, b)| same_command(a, b))
        {
            return false;
        }
        let usage = std::mem::take(&mut self.usage);
        let recency = std::mem::take(&mut self.recency);
        *self = Self::with_disabled(commands, self.include_disabled);
        self.usage = usage;
        self.prepare_recency(&recency);
        true
    }
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        #[cfg(test)]
        WORK.with(|work| { let (index, recency, search) = work.get(); work.set((index, recency, search + 1)); });
        let _timing = crate::performance_trace::shared().scope(crate::performance_trace::Stage::SearchModel);
        let mut results = if query.is_empty() {
            let mut order: Vec<_> = (0..self.commands.len()).collect();
            if !self.usage.is_empty() {
                // Stable sort retains catalog order within equal usage counts.
                order.sort_by_key(|&i| {
                    std::cmp::Reverse(*self.usage.get(&self.commands[i].id).unwrap_or(&0))
                });
            }
            order
        } else {
            self.index
                .search_with_recency(query, &self.weights)
                .expect("weights are prepared for this immutable catalog")
        };
        if limit > 0 {
            results.truncate(limit);
        }
        results
    }
}

/// Existing successful-choice policy: 512 entries, 90 days, no query text.
pub fn bounded_recency(entries: HashMap<String, f64>, now: f64) -> HashMap<String, f64> {
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|(id, time)| {
            !id.is_empty()
                && id.len() <= 512
                && time.is_finite()
                && *time >= 0.
                && *time > now - 90. * 86400.
                && *time <= now + 60.
        })
        .collect();
    entries.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.truncate(512);
    entries.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::fixture;
    #[test]
    fn prepared_ids_stay_aligned_after_filtering_and_reordering() {
        let mut commands = fixture(12);
        commands[0].enabled = Some(false);
        commands.reverse();
        let mut catalog = Catalog::new(commands);
        catalog.prepare_recency(&HashMap::from([("fixture.8".into(), 100.)]));
        let matches = catalog.search("microsoft", 0);
        assert_eq!(catalog.command(matches[0]).id, "fixture.8");
        assert!(!matches
            .iter()
            .any(|&i| catalog.command(i).id == "fixture.0"));
        catalog.prepare_recency(&HashMap::from([("fixture.4".into(), 200.)]));
        assert_eq!(
            catalog.command(catalog.search("microsoft", 0)[0]).id,
            "fixture.4"
        );
        assert_eq!(
            catalog.command(catalog.search("alias8", 0)[0]).id,
            "fixture.8"
        );
    }
    #[test]
    fn usage_only_reorders_empty_query_with_stable_ties() {
        let mut catalog = Catalog::new(fixture(12));
        let before = catalog.search("microsoft", 0);
        catalog.set_usage(HashMap::from([
            ("fixture.8".into(), 5),
            ("fixture.4".into(), 5),
        ]));
        let empty = catalog.search("", 2);
        assert_eq!(empty, vec![4, 8]);
        assert_eq!(catalog.search("microsoft", 0), before);
        assert_eq!(catalog.search(" ", 0), (0..12).collect::<Vec<_>>());
    }
    #[test]
    fn recency_bounds_and_ties_match_previous_policy() {
        let mut entries: HashMap<_, _> = (0..600).map(|n| (format!("{n:04}"), 1000.)).collect();
        for (id, time) in [
            ("future", 2000.),
            ("old", 0.),
            ("nan", f64::NAN),
            ("", 1000.),
        ] {
            entries.insert(id.into(), time);
        }
        let bounded = bounded_recency(entries, 1000. + 90. * 86400. - 1.);
        assert_eq!(bounded.len(), 512);
        assert!(!bounded.contains_key("old"));
        assert!(!bounded.contains_key("nan"));
        assert!(bounded.contains_key("0000"));
        assert!(!bounded.contains_key("0599"));
    }
    #[test]
    fn typed_ranked_entry_matches_existing_abi_and_rejects_bad_snapshots() {
        let commands = fixture(30);
        let index = SearchIndex::new(&commands);
        let weights: Vec<_> = (0..30).map(|n| if n == 12 { 100. } else { 0. }).collect();
        for query in ["", "m", "alias4", "missing Ω", "a\0b", "pro das"] {
            let mut output = vec![0usize; commands.len()];
            // SAFETY: all pointers borrow live separate buffers for this call.
            let count = unsafe {
                rustcast_core::rustcast_index_search_ranked_into(
                    &index,
                    query.as_ptr(),
                    query.len(),
                    weights.as_ptr(),
                    weights.len(),
                    output.as_mut_ptr(),
                    output.len(),
                )
            };
            assert!(count >= 0);
            output.truncate(count as usize);
            assert_eq!(index.search_with_recency(query, &weights).unwrap(), output);
        }
        assert!(index.search_with_recency("m", &[1.]).is_err());
        assert!(index.search_with_recency("m", &vec![f64::NAN; 30]).is_err());
        assert!(index.search_with_recency("m", &vec![-1.; 30]).is_err());
    }
}
