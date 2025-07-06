use thiserror::Error;
#[derive(Error, Debug)]
pub enum DiscordError {
    #[error("Discord error: {0}")]
    Discord(String),
}
pub type Result<T> = std::result::Result<T, DiscordError>;
