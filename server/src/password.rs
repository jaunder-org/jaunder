// Password lives in the `common` crate so both `web` and `server` can use it.
// Re-export for backward compatibility with existing server-crate consumers.
pub use common::password::{InvalidPassword, Password};
