//! Resolves everything under `~/Mnemos/`. The only place that knows the layout
//! (HLD §5.2). W4 extends this with ID-based conversation/project resolution.

use std::path::PathBuf;

use crate::error::AppError;

/// `~/Mnemos` — the app's single data root.
pub fn data_root() -> Result<PathBuf, AppError> {
    let home = home_dir().ok_or_else(|| AppError::internal("no home directory"))?;
    Ok(home.join("Mnemos"))
}

/// `~/Mnemos/logs`
pub fn logs_dir() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("logs"))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    let var = "HOME";
    #[cfg(windows)]
    let var = "USERPROFILE";

    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_dir_sits_under_the_data_root() {
        let logs = logs_dir().expect("home dir must resolve in test env");
        assert!(logs.ends_with("Mnemos/logs"));
    }
}
