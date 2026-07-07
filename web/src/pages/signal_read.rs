//! `read_signal!` reads a state signal reactively.
//!
//! Page components seed their signals from `Effect`s that run after hydration, so
//! a render closure reads reactively (`.get()`) to re-render once the Effect seeds
//! the signal. (`pages` compiles wasm-only since #300; before that this macro also
//! had a `.get_untracked()` arm for the never-mounted server build — the reactive
//! read is now unconditional.)

macro_rules! read_signal {
    ($signal:expr) => {{
        $signal.get()
    }};
}

pub(crate) use read_signal;
