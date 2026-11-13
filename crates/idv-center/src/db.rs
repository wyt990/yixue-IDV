use std::path::Path;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use chrono::Utc;
use rand_core::OsRng;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{SqlitePool, sqlite::SqliteRow};
use tracing::info;

use crate::models::{Admin, new_id};

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    if let Some(file) = sqlite_file_path(database_url)
        && let Some(parent) = Path::new(&file).parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let options: SqliteConnectOptions = database_url.parse()?;
    let options = options.create_if_missing(true).foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    seed_default_admin(&pool).await?;
    seed_default_modes(&pool).await?;
    Ok(pool)
}

async fn seed_default_modes(pool: &SqlitePool) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mode_policies")
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Ok(());
    }
    let now = Utc::now();
    let seeds = [
        (
            "teaching",
            "reboot",
            "restore",
            "keep",
            "wipe",
            "教学还原：开机还原系统区，保留数据区",
        ),
        (
            "office",
            "off",
            "keep",
            "keep",
            "keep",
            "办公模式：不自动还原",
        ),
        (
            "exam",
            "reboot",
            "restore",
            "keep",
            "wipe",
            "考试模式：开机还原系统区",
        ),
        (
            "manual",
            "manual",
            "restore",
            "keep",
            "wipe",
            "仅手动触发还原",
        ),
    ];
    for (name, restore, system, persistent, volatile, notes) in seeds {
        let usb = if name == "exam" { 1i64 } else { 0i64 };
        sqlx::query(
            "INSERT INTO mode_policies (id, name, restore, system_action, persistent_action, volatile_action, notes, schedule_start, schedule_end, usb_restricted, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(name)
        .bind(restore)
        .bind(system)
        .bind(persistent)
        .bind(volatile)
        .bind(notes)
        .bind(usb)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }
    info!("seeded default mode policies");
    Ok(())
}

async fn seed_default_admin(pool: &SqlitePool) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Ok(());
    }

    let password_hash = hash_password("admin123")?;
    let admin = Admin {
        id: new_id(),
        username: "admin".into(),
        password_hash,
        display_name: "系统管理员".into(),
        role: "super".into(),
        created_at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO admins (id, username, password_hash, display_name, role, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&admin.id)
    .bind(&admin.username)
    .bind(&admin.password_hash)
    .bind(&admin.display_name)
    .bind(&admin.role)
    .bind(admin.created_at)
    .execute(pool)
    .await?;

    info!("seeded default admin admin / admin123 — please change password later");
    Ok(())
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("password hash failed: {err}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    use argon2::PasswordVerifier;
    use password_hash::PasswordHash;
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Helper to keep row mapping imports tidy for future use.
#[allow(dead_code)]
pub type Row = SqliteRow;

fn sqlite_file_path(database_url: &str) -> Option<String> {
    let rest = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))?;
    let file = rest
        .split('?')
        .next()
        .unwrap_or(rest)
        .trim_start_matches('/');
    if file.is_empty() || file == ":memory:" {
        None
    } else {
        Some(file.to_string())
    }
}
