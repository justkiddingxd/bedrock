use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tokens (
    token_hash BLOB PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS vaults (
    id           TEXT PRIMARY KEY,
    owner_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key_envelope TEXT NOT NULL,
    head_seq     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

-- Only the latest version of each file is kept, so the change feed is
-- compacted for free: a client that was offline for a month downloads
-- each changed file once.
CREATE TABLE IF NOT EXISTS files (
    vault_id   TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    file_id    TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    deleted    INTEGER NOT NULL,
    blob       BLOB,
    size       INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (vault_id, file_id)
);

CREATE INDEX IF NOT EXISTS files_by_seq ON files(vault_id, seq);

-- People the owner shared a vault with. The owner is not listed here.
CREATE TABLE IF NOT EXISTS vault_members (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role     TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (vault_id, user_id)
);

-- Upcoming reminders for the Telegram bot: parsed from server vaults
-- (source 'server', key = file_id:...) or sent by apps for e2e vaults
-- (source 'client').
CREATE TABLE IF NOT EXISTS reminders (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    key      TEXT NOT NULL,
    at       INTEGER NOT NULL,
    text     TEXT NOT NULL,
    note     TEXT NOT NULL,
    source   TEXT NOT NULL,
    sent     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (vault_id, key)
);
CREATE INDEX IF NOT EXISTS reminders_due ON reminders(sent, at);

CREATE TABLE IF NOT EXISTS telegram_links (
    user_id        INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    chat_id        INTEGER NOT NULL,
    notify_changes INTEGER NOT NULL DEFAULT 1,
    linked_at      INTEGER NOT NULL
);

-- One-time codes behind t.me/<bot>?start=<code> links.
CREATE TABLE IF NOT EXISTS telegram_codes (
    code       TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

-- Bots people made themselves with @BotFather; the server runs them.
CREATE TABLE IF NOT EXISTS user_bots (
    user_id  INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    token    TEXT NOT NULL,
    username TEXT NOT NULL,
    added_at INTEGER NOT NULL
);

-- Where reminders are also sent as signed JSON, one URL per account.
CREATE TABLE IF NOT EXISTS reminder_webhooks (
    user_id          INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    url              TEXT NOT NULL,
    secret           TEXT NOT NULL,
    last_status      TEXT,
    last_delivery_at INTEGER,
    updated_at       INTEGER NOT NULL
);

-- Webhook deliveries: 'pending' until 'delivered' or given up ('failed').
-- One row per event and account, so an event is never queued twice.
CREATE TABLE IF NOT EXISTS webhook_queue (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id   TEXT NOT NULL,
    body       TEXT NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    next_at    INTEGER NOT NULL,
    state      TEXT NOT NULL DEFAULT 'pending',
    created_at INTEGER NOT NULL,
    UNIQUE (user_id, event_id)
);
CREATE INDEX IF NOT EXISTS webhook_queue_due ON webhook_queue(state, next_at);
"#;

/// Columns added after the first release, for databases created before them.
const COLUMNS: &[(&str, &str, &str)] = &[
    // 'e2e' or 'server'; server vaults have an empty key envelope.
    ("vaults", "mode", "TEXT NOT NULL DEFAULT 'e2e'"),
    // Path of a file in a server vault (NULL for e2e vaults).
    ("files", "path", "TEXT"),
    // Named API tokens; NULL for login sessions.
    ("tokens", "name", "TEXT"),
    ("users", "tz_offset", "INTEGER NOT NULL DEFAULT 0"),
    // Whose bot a chat or code belongs to: a user id, or NULL for the server's bot.
    ("telegram_links", "bot_owner", "INTEGER"),
    ("telegram_codes", "bot_owner", "INTEGER"),
];

pub async fn connect(path: &Path) -> sqlx::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(10))
        .foreign_keys(true);
    SqlitePoolOptions::new().max_connections(8).connect_with(opts).await
}

pub async fn migrate(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::raw_sql(SCHEMA).execute(pool).await?;
    for (table, column, decl) in COLUMNS {
        let exists = sqlx::query(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?"))
            .bind(column)
            .fetch_optional(pool)
            .await?
            .is_some();
        if !exists {
            sqlx::raw_sql(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}")).execute(pool).await?;
        }
    }
    sqlx::raw_sql("CREATE INDEX IF NOT EXISTS files_by_path ON files(vault_id, path)").execute(pool).await?;
    Ok(())
}
