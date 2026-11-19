//! The last of a sequence of requests whose results arrive in any order: each request takes the
//! next generation, and a result is kept only while it is the latest request's. The host's
//! `fonts` messages go through one, so the last message wins however long each one's index fetches
//! take (RFC-0034: each message gives the host's whole set).

/// The latest request's result, once it arrives ([`Latest::take`]).
pub struct Latest<T> {
    started: u64,
    ready: Option<T>,
}

impl<T> Default for Latest<T> {
    fn default() -> Self {
        Latest { started: 0, ready: None }
    }
}

impl<T> Latest<T> {
    /// A new request: its generation, which its result is given with. A result not taken yet of an
    /// earlier request is dropped.
    pub fn start(&mut self) -> u64 {
        self.started += 1;
        self.ready = None;
        self.started
    }

    /// The result of request `generation`: kept when it is the latest request's, else dropped.
    /// Returns whether it was kept.
    pub fn finish(&mut self, generation: u64, value: T) -> bool {
        if generation != self.started {
            return false;
        }
        self.ready = Some(value);
        true
    }

    /// [`Latest::finish`], asking `ctx` for a frame when the result is kept, so the app takes it
    /// at once rather than at the next input. Returns whether it was kept.
    pub fn finish_and_wake(&mut self, generation: u64, value: T, ctx: &egui::Context) -> bool {
        let kept = self.finish(generation, value);
        if kept {
            ctx.request_repaint();
        }
        kept
    }

    /// The latest request's result, once, when it has arrived.
    pub fn take(&mut self) -> Option<T> {
        self.ready.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request's result that arrives after a later request started is dropped, and the later
    /// one's is kept, in whichever order they arrive.
    #[test]
    fn the_last_request_wins_whatever_order_results_arrive_in() {
        let mut latest = Latest::default();
        let (first, second) = (latest.start(), latest.start());
        assert!(latest.finish(second, "second"));
        assert!(!latest.finish(first, "first"), "an older request's late result is dropped");
        assert_eq!(latest.take(), Some("second"));
        assert_eq!(latest.take(), None, "taken once");

        let first = latest.start();
        assert!(latest.finish(first, "early"));
        let second = latest.start();
        assert_eq!(latest.take(), None, "a result not taken is dropped when a later request starts");
        assert!(latest.finish(second, "late"));
        assert_eq!(latest.take(), Some("late"));
    }

    /// A result kept asks for a frame, so a refused set's error shows at once (review round 2,
    /// item 8); a dropped one doesn't.
    #[test]
    fn a_kept_result_asks_for_a_frame() {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        }
        assert!(!ctx.has_requested_repaint(), "idle to begin with");
        let mut latest = Latest::default();
        let (old, new) = (latest.start(), latest.start());
        assert!(!latest.finish_and_wake(old, "old", &ctx));
        assert!(!ctx.has_requested_repaint(), "a dropped result asks for nothing");
        assert!(latest.finish_and_wake(new, "new", &ctx));
        assert!(ctx.has_requested_repaint(), "a kept one asks for a frame");
    }
}
