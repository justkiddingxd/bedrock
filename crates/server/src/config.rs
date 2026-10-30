use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Registration {
    /// Anyone can create an account (shared server).
    Open,
    /// Only the first account can be created (personal server).
    FirstOnly,
    /// No new accounts.
    Closed,
}

#[derive(Debug, Clone, clap::Parser)]
#[command(name = "bedrock-server", version, about = "Bedrock sync server")]
pub struct Config {
    /// Address to listen on.
    #[arg(long, env = "BEDROCK_BIND", default_value = "0.0.0.0:8787")]
    pub bind: SocketAddr,

    /// Directory for the database.
    #[arg(long = "data", env = "BEDROCK_DATA", default_value = "./data")]
    pub data_dir: PathBuf,

    /// Who may create accounts.
    #[arg(long, env = "BEDROCK_REGISTRATION", value_enum, default_value = "first-only")]
    pub registration: Registration,

    /// Largest accepted file, in megabytes (after encryption).
    #[arg(long, env = "BEDROCK_MAX_FILE_MB", default_value_t = 64)]
    pub max_file_mb: u64,

    /// Name shown to clients.
    #[arg(long, env = "BEDROCK_NAME", default_value = "Bedrock Sync")]
    pub name: String,

    /// Token of a Telegram bot (from @BotFather) for reminders and notifications.
    #[arg(long, env = "BEDROCK_TELEGRAM_TOKEN")]
    pub telegram_token: Option<String>,

    /// Telegram Bot API address (for a local Bot API server or tests).
    #[arg(long, env = "BEDROCK_TELEGRAM_API", default_value = "https://api.telegram.org", hide = true)]
    pub telegram_api: String,

    /// How often due reminders are checked, in seconds.
    #[arg(long, default_value_t = 15, hide = true)]
    pub reminder_tick_secs: u64,

    /// Quiet time before edits in a shared vault are reported, in seconds.
    #[arg(long, default_value_t = 60, hide = true)]
    pub change_delay_secs: u64,

    /// Hosts reminder webhooks may reach although they are local or in a
    /// private network (comma-separated, e.g. 127.0.0.1). Others are refused.
    #[arg(long, env = "BEDROCK_WEBHOOK_ALLOW_HOSTS", value_delimiter = ',')]
    pub webhook_allow_hosts: Vec<String>,

    /// Waits before repeating a failed webhook delivery, in seconds.
    #[arg(long, value_delimiter = ',', default_values_t = [30, 120, 600], hide = true)]
    pub webhook_retry_secs: Vec<u64>,

    /// Timeout of one webhook request, in seconds.
    #[arg(long, default_value_t = 10, hide = true)]
    pub webhook_timeout_secs: u64,
}

impl Config {
    /// Defaults for embedding the server (tests, other binaries).
    pub fn new(bind: SocketAddr, data_dir: PathBuf, registration: Registration) -> Self {
        Self {
            bind,
            data_dir,
            registration,
            max_file_mb: 64,
            name: "Bedrock Sync".into(),
            telegram_token: None,
            telegram_api: "https://api.telegram.org".into(),
            reminder_tick_secs: 15,
            change_delay_secs: 60,
            webhook_allow_hosts: Vec::new(),
            webhook_retry_secs: vec![30, 120, 600],
            webhook_timeout_secs: 10,
        }
    }
}
