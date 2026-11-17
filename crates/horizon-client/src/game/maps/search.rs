//! One bounded worker per world's loot maps. The game thread only enqueues and
//! polls; even an absent structure in a heavily modded world cannot block it.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

type Job<T> = (String, Box<dyn FnOnce(&AtomicBool) -> T + Send>);

pub(super) struct Searches<T> {
    send: SyncSender<Job<T>>,
    receive: Receiver<(String, T)>,
    pending: HashMap<String, Option<T>>,
    stop: Arc<AtomicBool>,
}
impl<T: Send + 'static> Default for Searches<T> {
    fn default() -> Self {
        let (send, jobs) = mpsc::sync_channel::<Job<T>>(8);
        let (done, receive) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = stop.clone();
        std::thread::Builder::new()
            .name("explorer-maps".into())
            .spawn(move || {
                while let Ok((key, job)) = jobs.recv() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = job(&cancel);
                    if done.send((key, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("start explorer map worker");
        Self {
            send,
            receive,
            pending: HashMap::new(),
            stop,
        }
    }
}
impl<T> Searches<T> {
    pub(super) fn contains(&self, key: &str) -> bool {
        self.pending.contains_key(key)
    }

    /// A full queue leaves the saved item pending for a later tick/open.
    pub(super) fn queue(
        &mut self,
        key: String,
        job: impl FnOnce(&AtomicBool) -> T + Send + 'static,
    ) {
        if self.contains(&key) {
            return;
        }
        if self.send.try_send((key.clone(), Box::new(job))).is_ok() {
            self.pending.insert(key, None);
        }
    }
    pub(super) fn take_ready(&mut self, key: &str) -> Option<T> {
        while let Ok((key, result)) = self.receive.try_recv() {
            self.pending.insert(key, Some(result));
        }
        let result = self.pending.get_mut(key)?.take()?;
        self.pending.remove(key);
        Some(result)
    }
}
impl<T> Drop for Searches<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_blocked_search_does_not_block_polling_or_duplicate_work() {
        let mut searches = Searches::default();
        let (release, wait) = mpsc::channel();
        let (started, running) = mpsc::channel();
        searches.queue("map".into(), move |_| {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            42
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        searches.queue("map".into(), |_| panic!("duplicate search"));
        assert_eq!(searches.take_ready("map"), None);
        assert!(searches.contains("map"));
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = searches.take_ready("map") {
                assert_eq!(result, 42);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn closing_a_world_cancels_running_and_queued_searches_without_joining() {
        let mut searches = Searches::default();
        let (started, running) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        searches.queue("running".into(), move |stop| {
            started.send(()).unwrap();
            while !stop.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
            finished.send(()).unwrap();
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        searches.queue("queued".into(), |_| panic!("cancelled queue ran"));
        drop(searches);
        done.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn queue_saturation_leaves_additional_items_pending_for_retry() {
        let mut searches = Searches::default();
        let (started, running) = mpsc::channel();
        searches.queue("running".into(), move |stop| {
            started.send(()).unwrap();
            while !stop.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        for i in 0..8 {
            searches.queue(i.to_string(), |_| ());
        }
        searches.queue("retry".into(), |_| ());
        assert!(!searches.contains("retry"));
        assert_eq!(searches.pending.len(), 9);
    }
}
