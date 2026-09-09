use rust_embed::RustEmbed;

#[derive(RustEmbed, Clone)]
// cov:ignore: RustEmbed derive expansion is compiler-generated rather than handwritten runtime behavior.
#[folder = "assets/"]
pub struct StaticAssets;
