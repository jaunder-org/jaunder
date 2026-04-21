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
