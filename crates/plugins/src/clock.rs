//! The monotonic clock of run deadlines and the deferred view's budget: `std::time::Instant` on
//! the desktop; on wasm32 in a browser (`target_os = "unknown"`, web-time's own test) the binary
//! picks it, as getrandom asks libraries to leave its source to the binary. The web build turns on
//! `web-clock` (the page's `performance.now()` through web-time); an embedder without JavaScript,
//! whose WebAssembly module must have no imports, has no clock: a run then has no wall-clock
//! deadline (its fuel and memory limits still hold, and the embedder's own limits on CPU time), and
//! [`crate::deferred::Clock`] reads `None`.

pub(crate) use imp::{Instant, now};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod imp {
    pub(crate) type Instant = std::time::Instant;

    /// Now on the system's monotonic clock.
    pub(crate) fn now() -> Option<Instant> {
        Some(Instant::now())
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown", feature = "web-clock"))]
mod imp {
    pub(crate) type Instant = web_time::Instant;

    /// Now on the page's clock (`performance.now()`).
    pub(crate) fn now() -> Option<Instant> {
        Some(Instant::now())
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown", not(feature = "web-clock")))]
mod imp {
    use std::time::Duration;

    /// No instant can be made without a clock: the type has no values, so nothing can read one
    /// (`std::time::Instant::now` would panic in such a module).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(crate) enum Instant {}

    impl Instant {
        pub(crate) fn checked_add(&self, _: Duration) -> Option<Self> {
            match *self {}
        }

        pub(crate) fn saturating_duration_since(&self, _: Self) -> Duration {
            match *self {}
        }
    }

    /// There is no clock to read.
    pub(crate) fn now() -> Option<Instant> {
        None
    }
}

#[cfg(test)]
mod tests {
    /// The desktop always has a clock, and it moves forward.
    #[test]
    fn the_desktop_reads_a_monotonic_clock() {
        let a = super::now().unwrap();
        let b = super::now().unwrap();
        assert!(b >= a);
    }

    /// The web build gives plug-ins the page's clock, and no library turns web-time on for every
    /// wasm32 build: an embedder without JavaScript must be able to build a module with no imports.
    #[test]
    fn the_clock_on_wasm32_is_the_binarys_to_pick() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains(r#"web-time = { version = "1", optional = true }"#), "vectorcraft-plugins turns web-time on itself");
        let web = include_str!("../../../apps/vectorcraft-web/Cargo.toml");
        assert!(web.contains(r#"features = ["web-clock"]"#), "the web app gives plug-ins no clock");
    }
}
