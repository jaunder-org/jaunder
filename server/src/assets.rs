use rust_embed::RustEmbed;

#[derive(RustEmbed, Clone)] // cov:ignore
#[folder = "assets/"]
pub struct StaticAssets;
