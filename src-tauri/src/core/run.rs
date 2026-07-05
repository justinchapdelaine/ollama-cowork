use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::core::error::{AppError, AppResult};

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Default)]
pub struct CancellationFlag {
    cancelled: Arc<AtomicBool>,
}

impl CancellationFlag {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn check(&self) -> AppResult<()> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(AppError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub async fn cancelled(&self) {
        while !self.cancelled.load(Ordering::SeqCst) {
            sleep(CANCEL_POLL_INTERVAL).await;
        }
    }
}

#[derive(Debug, Default)]
pub struct AgentRunStore {
    runs: Mutex<HashMap<Uuid, CancellationFlag>>,
}

impl AgentRunStore {
    pub fn begin(&self, run_id: Uuid) -> AppResult<CancellationFlag> {
        let flag = CancellationFlag::default();
        self.runs
            .lock()
            .map_err(|err| AppError::Runtime(format!("agent run store poisoned: {err}")))?
            .insert(run_id, flag.clone());
        Ok(flag)
    }

    pub fn cancel(&self, run_id: Uuid) -> AppResult<bool> {
        let runs = self
            .runs
            .lock()
            .map_err(|err| AppError::Runtime(format!("agent run store poisoned: {err}")))?;
        let Some(flag) = runs.get(&run_id) else {
            return Ok(false);
        };

        flag.cancel();
        Ok(true)
    }

    pub fn finish(&self, run_id: Uuid) -> AppResult<()> {
        self.runs
            .lock()
            .map_err(|err| AppError::Runtime(format!("agent run store poisoned: {err}")))?
            .remove(&run_id);
        Ok(())
    }
}
