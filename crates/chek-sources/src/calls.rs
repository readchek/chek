//! Reader work goes ahead of queued catalog calls, without sharing a live browser.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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

/// One browser per extension and user at a time. Calls run in parallel, and
/// a site block can reach several at once; each starting its own browser to
/// pass the same check costs a Chromium apiece and solves it several times.
/// The rest wait here, then find the cookies the first one earned.
#[derive(Default)]
pub(crate) struct Browsers {
    open: Mutex<bool>,
    closed: Condvar,
}

impl Browsers {
    /// Waits up to `wait` for the browser to be free and takes it.
    pub(crate) fn take(&self, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        while *open {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("another call is still using this source's browser; retry".into());
            }
            open = self
                .closed
                .wait_timeout(open, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *open = true;
        Ok(())
    }

    pub(crate) fn give_back(&self) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.closed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_that_need_a_browser_take_turns() {
        let browsers = Arc::new(Browsers::default());
        browsers.take(Duration::from_secs(1)).unwrap();
        assert!(browsers.take(Duration::from_millis(20)).is_err());

        let waiting = {
            let browsers = Arc::clone(&browsers);
            std::thread::spawn(move || browsers.take(Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(50));
        browsers.give_back();
        assert!(waiting.join().unwrap().is_ok());
        assert!(browsers.take(Duration::from_millis(20)).is_err());
    }
}
