pub mod file_capture;
pub mod http;
pub mod noop;

pub use file_capture::FileCapturingWebSubClient;
pub use http::HttpWebSubClient;
pub use noop::NoopWebSubClient;

mod contract;
pub use contract::{WebSubClient, WebSubError};
mod factory;
pub use factory::default_client;
