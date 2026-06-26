//! `read_signal!` reads a state signal the way each render target needs it.
//!
//! Page components seed their signals from wasm-only `Effect`s that run after
//! hydration and never fire during SSR. So a render closure must read reactively
//! on the client (`.get()`), to re-render once the Effect seeds the signal, but
//! read once and untracked on the server (`.get_untracked()`), where nothing
//! will ever update it and a tracked read only registers a subscription that can
//! never fire.

macro_rules! read_signal {
    ($signal:expr) => {{
        #[cfg(target_arch = "wasm32")]
        {
            $signal.get()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            $signal.get_untracked()
        }
    }};
}

pub(crate) use read_signal;
