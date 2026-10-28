//! Reader work goes ahead of queued catalog calls, without sharing a live browser.
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// Calls one extension may run at once. Each waits mostly on the network, so
/// a few overlap well.
const PARALLEL: usize = 4;

pub(crate) struct Calls {
    state: Mutex<State>,
    ready: Condvar,
    /// Set while a person has a verification browser open. Calls queue then,
    /// so their cookies can't replace the ones the person's browser earns.
    verification: Arc<Verification>,
}

/// Whether a person has a verification browser open. It lapses with that
/// browser, so one left open and abandoned doesn't hold calls back forever.
#[derive(Default)]
pub(crate) struct Verification(Mutex<Option<Instant>>);

impl Verification {
    /// The person's browser is open until `until`.
    pub(crate) fn start(&self, until: Instant) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
    }

    pub(crate) fn end(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(crate) fn pending(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|until| Instant::now() < until)
    }
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
    pub(crate) fn new(verification: Arc<Verification>) -> Self {
        Self {
            state: Mutex::default(),
            ready: Condvar::new(),
            verification,
        }
    }

    fn limit(&self) -> usize {
        if self.verification.pending() {
            1
        } else {
            PARALLEL
        }
    }

    pub(crate) fn lock(&self, reader: bool) -> Call<'_> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if reader {
            state.readers += 1;
        } else {
            state.catalog += 1;
        }
        while state.active >= self.limit()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_abandoned_verification_browser_stops_holding_calls_back() {
        let verification = Arc::new(Verification::default());
        let calls = Calls::new(Arc::clone(&verification));
        assert_eq!(calls.limit(), PARALLEL);

        verification.start(Instant::now() + Duration::from_secs(600));
        assert!(verification.pending());
        assert_eq!(calls.limit(), 1);

        // The person's browser closed on its own without a resume or close.
        verification.start(Instant::now() - Duration::from_secs(1));
        assert!(!verification.pending());
        assert_eq!(calls.limit(), PARALLEL);

        verification.start(Instant::now() + Duration::from_secs(600));
        verification.end();
        assert!(!verification.pending());
    }
}
