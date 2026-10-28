use std::path::PathBuf;

/// Where `scripts/fetch-models.sh` puts models. The app never downloads.
pub fn dir() -> PathBuf {
    std::env::var_os("SUNDAY_MODELS").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache/sunday/models")
    })
}

pub fn path(rel: &str) -> String {
    dir().join(rel).to_string_lossy().into_owned()
}
