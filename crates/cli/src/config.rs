//! Runtime settings for the rutter binary.
//!
//! Boundary: flag and environment resolution only. The CLI is
//! configured exclusively through flags and the environment; the
//! policy file is passed as `--policy`. Every default is overridable;
//! nothing is read from the network here.

use std::path::PathBuf;
use std::time::Duration;

use rutter_engine::error::EngineError;

/// Deadline for one navigation in `open` mode.
const DEFAULT_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolved settings for one invocation.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Engine binary the caller insists on; bypasses download and cache.
    pub engine_executable: Option<PathBuf>,
    /// Root of the engine cache (default: OS cache dir + `rutter`).
    pub cache_root: PathBuf,
    /// Arguments passed verbatim to the engine process.
    pub extra_engine_args: Vec<String>,
    /// Deadline for one navigation.
    pub navigation_timeout: Duration,
}

impl Settings {
    /// Builds settings from explicit flag values and the environment.
    pub fn resolve(
        engine_executable: Option<PathBuf>,
        cache_dir: Option<PathBuf>,
        extra_engine_args: Vec<String>,
    ) -> Result<Self, EngineError> {
        let cache_root = match cache_dir {
            Some(dir) => dir,
            None => rutter_engine::download::cache_root_default()?,
        };
        Ok(Self {
            engine_executable,
            cache_root,
            extra_engine_args,
            navigation_timeout: DEFAULT_NAVIGATION_TIMEOUT,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_values_win_over_defaults() {
        let settings = Settings::resolve(
            Some(PathBuf::from("browser.exe")),
            Some(PathBuf::from("cache")),
            Vec::new(),
        )
        .expect("resolve");
        assert_eq!(
            settings.engine_executable,
            Some(PathBuf::from("browser.exe"))
        );
        assert_eq!(settings.cache_root, PathBuf::from("cache"));
        assert_eq!(settings.navigation_timeout, DEFAULT_NAVIGATION_TIMEOUT);
    }
}
