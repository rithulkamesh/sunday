#[cfg(feature = "voice")]
pub mod provider;
#[cfg(feature = "voice")]
pub mod applefm;
#[cfg(all(feature = "voice", feature = "mlx", target_os = "macos"))]
pub mod mlx;
#[cfg(feature = "voice")]
pub mod localembed;
