use std::env;

/// Errors that can occur while reading the bot configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable `{0}` is not set")]
    Missing(&'static str),

    #[error("environment variable `{0}` is not valid UTF-8")]
    NotUnicode(&'static str),

    #[error("environment variable `{name}` must be an integer, got `{value}`")]
    NotAnInteger { name: &'static str, value: String },
}

/// Reads the environment variable `name`, failing if it is unset or not valid UTF-8.
pub fn env_var(name: &'static str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(ConfigError::Missing(name)),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::NotUnicode(name)),
    }
}

/// Reads the environment variable `name` and parses it as an `i64`.
pub fn env_var_i64(name: &'static str) -> Result<i64, ConfigError> {
    let value = env_var(name)?;
    value
        .parse()
        .map_err(|_| ConfigError::NotAnInteger { name, value })
}
