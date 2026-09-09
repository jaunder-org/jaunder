use rust_embed::RustEmbed;

// cov:ignore-start: RustEmbed derive expansion is compiler-generated rather than handwritten runtime behavior.
#[derive(RustEmbed, Clone)]
// cov:ignore-stop
#[folder = "assets/"]
pub struct StaticAssets;
