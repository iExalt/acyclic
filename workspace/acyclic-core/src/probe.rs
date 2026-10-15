//! Timing marks for the store benchmark. With the `probe` feature, each mark prints to standard error how long the
//! process spent since the previous mark, so a mark names the work that just ended; without it, marks compile to
//! nothing. Builds that ship never enable it.

/// The prefix of a mark's line: `acyclic-probe<TAB>name<TAB>microseconds`.
pub const PREFIX: &str = "acyclic-probe";

/// Marks the end of the work named `name`.
#[cfg(feature = "probe")]
pub fn mark(name: &str) {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let now = Instant::now();
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let since = last.map_or(0, |last| (now - last).as_micros());
    *last = Some(now);
    eprintln!("{PREFIX}\t{name}\t{since}");
}

/// Marks the end of the work named `name`.
#[cfg(not(feature = "probe"))]
#[inline(always)]
pub fn mark(_name: &str) {}
