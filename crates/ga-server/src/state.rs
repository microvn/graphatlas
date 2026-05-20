//! AppState — shared handles. Cloneable via Arc.

use std::sync::Arc;

use crate::config::ServerConfig;
use crate::data::ProjectDataSource;
use crate::jobs::{ConfirmTokens, JobLauncher, JobRegistry};
use crate::watcher::{WatcherDriver, WatcherRegistry};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<ServerConfig>,
    pub jobs: Arc<JobRegistry>,
    pub confirm_tokens: Arc<ConfirmTokens>,
    pub launcher: Arc<dyn JobLauncher>,
    pub data: Arc<dyn ProjectDataSource>,
    pub watchers: Arc<WatcherRegistry>,
    pub watcher_driver: Arc<dyn WatcherDriver>,
}

impl AppState {
    pub fn new(
        cfg: ServerConfig,
        launcher: Arc<dyn JobLauncher>,
        data: Arc<dyn ProjectDataSource>,
        watcher_driver: Arc<dyn WatcherDriver>,
    ) -> Self {
        Self {
            cfg: Arc::new(cfg),
            jobs: Arc::new(JobRegistry::new()),
            confirm_tokens: Arc::new(ConfirmTokens::new()),
            launcher,
            data,
            watchers: Arc::new(WatcherRegistry::new()),
            watcher_driver,
        }
    }
}
