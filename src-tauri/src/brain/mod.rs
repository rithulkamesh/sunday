#[cfg(feature = "voice")]
pub mod llm;
#[cfg(feature = "voice")]
pub mod decision;
#[cfg(feature = "voice")]
pub mod laya;
#[cfg(feature = "voice")]
pub mod corrections;
#[cfg(feature = "voice")]
pub mod learn;
#[cfg(feature = "voice")]
pub mod skills;
#[cfg(all(test, feature = "voice"))]
pub mod eval;
