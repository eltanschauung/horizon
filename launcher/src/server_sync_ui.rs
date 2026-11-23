//! UI for the server-pinned update job; one worker owns download/build mutations.
use super::*;
use horizon_launcher::server_sync::{self, Phase};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct SyncUi {
    running: Option<Run>,
    checked: HashMap<String, Instant>,
    failed: HashMap<String, String>,
    launch_ready: Option<String>,
    preparing: Option<(String, bool)>,
    play_after: bool,
}
struct Run {
    id: String,
    phase: Phase,
    started: Instant,
    elapsed: [Duration; 2],
    shown: u64,
    detail: String,
}
fn index(p: Phase) -> usize {
    match p {
        Phase::Downloading => 0,
        Phase::Building => 1,
    }
}
impl Run {
    fn elapsed(&self) -> Duration {
        self.elapsed[index(self.phase)] + self.started.elapsed()
    }
    fn progress(&mut self, phase: Phase, detail: String) {
        if phase != self.phase {
            self.elapsed[index(self.phase)] += self.started.elapsed();
            self.phase = phase;
            self.started = Instant::now();
        }
        self.detail = detail;
    }
    fn label(&self) -> String {
        let t = self.elapsed().as_secs();
        format!("{} {}:{:02}", self.phase.label(), t / 60, t % 60)
    }
}
impl Launcher {
    fn managed_instance(&self) -> Option<instances::Instance> {
        self.selected_instance()
            .filter(|i| server_sync::manages_instance(i))
            .cloned()
    }
    pub(super) fn server_sync_status(&self) -> Option<String> {
        self.managed_instance()?;
        self.server_sync.running.as_ref().map(Run::label)
    }
    pub(super) fn server_sync_error(&self) -> Option<&str> {
        let id = &self.selected_instance()?.id;
        self.server_sync.failed.get(id).map(String::as_str)
    }

    fn server_sync_failed(&mut self, id: String, error: String) {
        self.client_error = Some(error.clone());
        self.server_sync.failed.insert(id, error);
    }

    pub(super) fn begin_server_sync(&mut self, play_after: bool) {
        if self.server_sync.running.is_some() {
            self.server_sync.play_after |= play_after;
            return;
        }
        let Some(instance) = self.managed_instance() else {
            return;
        };
        if self.client.is_some()
            || self.starting_client.is_some()
            || self.native_step.is_some()
            || self.preparing
            || self.installing.is_some()
        {
            return;
        }
        if !self.settings.native_mods_allowed() {
            if play_after && self.pending_consent.is_none() {
                self.pending_consent = Some(crate::native_consent::PendingConsent::ServerSync);
            }
            return;
        }
        let id = instance.id.clone();
        let current = self.version_of(&instance).cloned();
        self.server_sync.failed.remove(&id);
        self.server_sync.checked.insert(id.clone(), Instant::now());
        self.server_sync.play_after = play_after;
        self.server_sync.running = Some(Run {
            id: id.clone(),
            phase: Phase::Downloading,
            started: Instant::now(),
            elapsed: [Duration::ZERO; 2],
            shown: 0,
            detail: String::new(),
        });
        self.client_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                server_sync::sync(&instance, current.as_deref(), |phase, line| {
                    let _ = tx.send(Msg::ServerSyncProgress(id.clone(), phase, line));
                }, |inventory| crate::native_registry_consent::request(&tx, inventory))
            }))
            .unwrap_or_else(|_| {
                Err("The server update stopped unexpectedly; press Play to retry".into())
            });
            let _ = tx.send(Msg::ServerSynced(id, result));
        });
    }
    pub(super) fn server_sync_before_play(&mut self) -> bool {
        let Some(instance) = self.managed_instance() else {
            return false;
        };
        if self.server_sync.launch_ready.take().as_deref() == Some(instance.id.as_str()) {
            return false;
        }
        if let Some((id, play)) = &mut self.server_sync.preparing {
            if *id == instance.id {
                *play = true;
            }
        } else {
            self.begin_server_sync(true);
        }
        true
    }
    pub(super) fn server_sync_tick(&mut self) -> bool {
        if let Some(run) = &mut self.server_sync.running {
            let elapsed = run.elapsed().as_secs();
            if elapsed != run.shown {
                run.shown = elapsed;
                return true;
            }
            return false;
        }
        let Some(i) = self.managed_instance() else {
            return false;
        };
        let stale = self
            .server_sync
            .checked
            .get(&i.id)
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
        if stale
            && self.pending_consent.is_none()
            && !self.server_sync.failed.contains_key(&i.id)
            && self.client.is_none()
            && self.starting_client.is_none()
            && self.native_step.is_none()
            && !self.preparing
            && self.installing.is_none()
        {
            self.begin_server_sync(false);
            return self.server_sync.running.is_some();
        }
        false
    }
    pub(super) fn server_sync_progress(&mut self, id: String, phase: Phase, line: String) {
        if let Some(run) = self.server_sync.running.as_mut().filter(|r| r.id == id) {
            run.progress(phase, line);
        }
    }
    pub(super) fn server_sync_prepared(&mut self, succeeded: bool) {
        let Some((id, play)) = self.server_sync.preparing.take() else {
            return;
        };
        if !succeeded {
            if let Some(error) = self.prepare_error.clone() {
                self.server_sync_failed(id, error);
            }
        } else if self
            .managed_instance()
            .is_some_and(|instance| instance.id == id)
        {
            self.begin_server_sync(play);
        }
    }

    pub(super) fn server_sync_done(
        &mut self,
        id: String,
        result: Result<server_sync::Outcome, String>,
    ) {
        if !self
            .server_sync
            .running
            .as_ref()
            .is_some_and(|r| r.id == id)
        {
            return;
        }
        self.server_sync.running = None;
        self.server_sync.checked.insert(id.clone(), Instant::now());
        self.installed = backend::installed();
        self.reload_instances();
        self.instance_mods_changed();
        self.native_log.stale = true;
        let play = std::mem::take(&mut self.server_sync.play_after);
        match result {
            Ok(server_sync::Outcome::Prepare(base)) => {
                if self.horizon_preparation_access() == Some(true) {
                    self.server_sync.preparing = Some((id, play));
                    self.prepare_version(base, backend::MinecraftFrom::Install);
                } else {
                    self.server_sync_failed(
                        id,
                        tr("The server's game build needs Minecraft assets. Sign in with Ely.by or a Microsoft account that owns Minecraft: Java Edition, then press Play.").into(),
                    );
                }
            }
            Ok(server_sync::Outcome::Ready(_))
                if play && self.managed_instance().is_some_and(|i| i.id == id) =>
            {
                self.server_sync.launch_ready = Some(id);
                self.play();
            }
            Ok(_) => {}
            Err(error) => self.server_sync_failed(id, format!("Server update failed: {error}")),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_clocks_accumulate_separately_when_downloads_resume_during_build() {
        let mut r = Run {
            id: "test".into(),
            phase: Phase::Downloading,
            started: Instant::now() - Duration::from_secs(3),
            elapsed: [Duration::ZERO; 2],
            shown: 0,
            detail: String::new(),
        };
        r.progress(Phase::Building, "compiler".into());
        assert!(r.elapsed[0] >= Duration::from_secs(3));
        r.started = Instant::now() - Duration::from_secs(2);
        r.progress(Phase::Downloading, "source".into());
        assert!(r.elapsed[1] >= Duration::from_secs(2));
        assert!(r.label().starts_with("Downloading Updates... 0:03"));
    }
}
