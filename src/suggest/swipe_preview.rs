use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use super::dictionary::Dictionary;
use super::swipe::SwipeKeyGeom;
use super::swipe::SwipeRecognizer;

pub struct PreviewRequest {
    pub touch_id: i32,
    pub sequence: u64,
    pub keys: Vec<SwipeKeyGeom>,
    pub aspect: f32,
    pub points: Vec<(f32, f32, f64)>,
    pub dictionary_revision: u64,
    pub dictionary_update: Option<Dictionary>,
}

pub struct PreviewResult {
    pub touch_id: i32,
    pub sequence: u64,
    pub dictionary_revision: u64,
    pub candidates: Vec<String>,
    pub elapsed_ms: f64,
}

#[derive(Default)]
struct Mailbox {
    pending: Option<PreviewRequest>,
    dictionary_update: Option<(u64, Dictionary)>,
    result: Option<PreviewResult>,
}

pub struct SwipePreviewWorker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
}

impl SwipePreviewWorker {
    pub fn new(initial_dictionary: Dictionary) -> Option<Self> {
        let mailbox = Arc::new((
            Mutex::new(Mailbox {
                dictionary_update: Some((
                    initial_dictionary.swipe_lexicon_revision(),
                    initial_dictionary,
                )),
                ..Mailbox::default()
            }),
            Condvar::new(),
        ));
        let worker_mailbox = Arc::clone(&mailbox);
        if let Err(error) = thread::Builder::new()
            .name("hyprosk-swipe-preview".into())
            .spawn(move || Self::run(worker_mailbox))
        {
            tracing::warn!(%error, "swipe preview worker could not start");
            return None;
        }
        Some(Self { mailbox })
    }

    pub fn submit(&self, mut request: PreviewRequest) {
        let (lock, wake) = &*self.mailbox;
        let mut mailbox = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(dictionary) = request.dictionary_update.take() {
            mailbox.dictionary_update = Some((request.dictionary_revision, dictionary));
        }
        // Replacing this slot coalesces bursts of motion into the latest path.
        mailbox.pending = Some(request);
        wake.notify_one();
    }

    pub fn take_result(&self) -> Option<PreviewResult> {
        let (lock, _) = &*self.mailbox;
        lock.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .result
            .take()
    }

    fn run(mailbox: Arc<(Mutex<Mailbox>, Condvar)>) {
        let mut recognizer: Option<SwipeRecognizer> = None;
        let mut dictionary: Option<Dictionary> = None;
        let mut dictionary_revision = None;
        loop {
            let request = {
                let (lock, wake) = &*mailbox;
                let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                while state.pending.is_none() {
                    state = wake
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                if let Some((revision, updated)) = state.dictionary_update.take() {
                    dictionary_revision = Some(revision);
                    dictionary = Some(updated);
                }
                state.pending.take().expect("pending request was checked")
            };

            let started = Instant::now();
            if recognizer.is_none() {
                recognizer = SwipeRecognizer::new();
            }
            let candidates = match (dictionary.as_ref(), recognizer.as_mut()) {
                (Some(dictionary), Some(engine)) => {
                    engine.prepare_layout(&request.keys);
                    engine.decode(&request.points, dictionary, request.aspect, 6)
                }
                _ => Vec::new(),
            };
            let result = PreviewResult {
                touch_id: request.touch_id,
                sequence: request.sequence,
                dictionary_revision: dictionary_revision.unwrap_or(request.dictionary_revision),
                candidates,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
            let (lock, _) = &*mailbox;
            lock.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .result = Some(result);
        }
    }
}
