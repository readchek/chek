//! Reader work goes ahead of queued catalog calls, without sharing a live browser.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Calls one extension may run at once. Each waits mostly on the network, so
/// a few overlap well.
const PARALLEL: usize = 4;

pub(crate) struct Calls {
    state: Mutex<State>,
    ready: Condvar,
    /// Set while a person has a verification browser open. Calls queue then,
    /// so their cookies can't replace the ones the person's browser earns.
    verification_pending: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    active: usize,
    readers: usize,
    catalog: usize,
    reader_streak: usize,
}

pub(crate) struct Call<'a>(&'a Calls);

impl Calls {
    pub(crate) fn new(verification_pending: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::default(),
            ready: Condvar::new(),
            verification_pending,
        }
    }

    fn limit(&self) -> usize {
        if self.verification_pending.load(Ordering::SeqCst) {
            1
        } else {
            PARALLEL
        }
    }

    pub(crate) fn lock(&self, reader: bool) -> Call<'_> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let limit = self.limit();
        if reader {
            state.readers += 1;
        } else {
            state.catalog += 1;
        }
        while state.active >= limit
            || (reader && state.catalog > 0 && state.reader_streak >= 3)
            || (!reader && state.readers > 0 && state.reader_streak < 3)
        {
            state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state.active += 1;
        if reader {
            state.readers -= 1;
            state.reader_streak = state.reader_streak.saturating_add(1);
        } else {
            state.catalog -= 1;
            state.reader_streak = 0;
        }
        Call(self)
    }

    /// The only call running, for the verification browser.
    pub(crate) fn try_lock(&self) -> Result<Call<'_>, ()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active > 0 || state.readers > 0 || state.catalog > 0 {
            return Err(());
        }
        state.active = 1;
        state.reader_streak = 0;
        Ok(Call(self))
    }
}

impl Drop for Call<'_> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active -= 1;
        self.0.ready.notify_all();
    }
}
