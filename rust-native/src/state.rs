//! Rust composition root for persisted configuration and the launcher model.
//! Native effects/hotkey staging are deliberately not started by construction.
use crate::{
    profile::{ConfigStore, ProfilePaths},
    ranking::RankingStore,
    session::Session,
};

pub struct HostState {
    pub editing: crate::actions::Editing,
    pub configuration: ConfigStore,
    pub session: Session,
    pub ranking: RankingStore,
}
impl HostState {
    pub fn open(paths: ProfilePaths) -> Result<Self, String> {
        let configuration = ConfigStore::open(paths)?;
        let mut session = Session::new(configuration.current().commands.clone());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs_f64();
        let ranking = RankingStore::open(&configuration.paths.root, now);
        session.catalog.prepare_recency(ranking.entries());
        Ok(Self {
            editing: crate::actions::Editing::default(),
            configuration,
            session,
            ranking,
        })
    }
    pub fn record_success(
        &mut self,
        id: &str,
        now: f64,
    ) -> Result<crate::profile::SaveReceipt, String> {
        let receipt = self.ranking.record_success(id, now)?;
        self.session.catalog.prepare_recency(self.ranking.entries());
        self.session.refresh_preserving_selection();
        Ok(receipt)
    }
    /// Last-good validation must succeed before the active catalog changes.
    /// The eventual runtime adds staged hotkey/effect reconciliation here.
    pub fn reload(&mut self) -> Result<bool, String> {
        self.reload_limited(None)
    }
    pub fn reload_watched(&mut self) -> Result<bool, String> {
        self.reload_limited(Some(32_000_000))
    }
    fn reload_limited(&mut self, limit: Option<usize>) -> Result<bool, String> {
        let changed = self.configuration.reload_limited(limit)?;
        if changed {
            self.session
                .replace_commands(self.configuration.current().commands.clone());
        }
        Ok(changed)
    }
}
