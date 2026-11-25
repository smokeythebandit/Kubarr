//! Destination verification codes for user notification preferences.
//!
//! A user proves they own a destination (email address, Telegram chat, phone
//! number) by echoing back a 6-digit code sent to it. Only a salted SHA-256 of
//! the code is stored, bound to the channel and destination, so a code issued
//! for an old destination can never verify a new one. Each code expires after
//! [`CODE_TTL_SECS`] and allows [`MAX_ATTEMPTS`] guesses; sends are limited to
//! one per [`RESEND_INTERVAL_SECS`] and [`MAX_SENDS_PER_HOUR`] per hour.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use rand::RngExt;
use rsa::sha2::{Digest, Sha256};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
};

use crate::error::{AppError, Result};
use crate::models::notification_verification::{self, Column, Entity};

/// How long a code stays valid.
pub const CODE_TTL_SECS: i64 = 10 * 60;
/// Guesses allowed per code.
pub const MAX_ATTEMPTS: i32 = 5;
/// Minimum gap between two sends for one user and channel.
pub const RESEND_INTERVAL_SECS: i64 = 60;
/// Sends allowed per rolling hour window for one user and channel.
pub const MAX_SENDS_PER_HOUR: i32 = 5;

const WINDOW_SECS: i64 = 60 * 60;

/// Message title used for verification codes.
pub const CODE_TITLE: &str = "Kubarr verification code";

/// Message body carrying `code`.
pub fn code_message(code: &str) -> String {
    format!("Your Kubarr verification code is {code}")
}

/// A uniformly random 6-digit code from the thread-local CSPRNG.
pub fn generate_code() -> String {
    let n: u32 = rand::rng().random_range(0..1_000_000);
    format!("{n:06}")
}

fn digest(salt: &str, channel_type: &str, destination: &str, code: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [salt, channel_type, destination, code] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hex::encode(hasher.finalize())
}

/// Salted hash of `code`, bound to the channel and destination: `salt_hex$sha256_hex`.
pub fn hash_code(channel_type: &str, destination: &str, code: &str) -> String {
    let salt = crate::services::security::generate_random_string(16);
    let hash = digest(&salt, channel_type, destination, code);
    format!("{salt}${hash}")
}

/// Constant-time check of `code` against a stored [`hash_code`] value.
pub fn code_matches(stored: &str, channel_type: &str, destination: &str, code: &str) -> bool {
    let Some((salt, expected)) = stored.split_once('$') else {
        return false;
    };
    let actual = digest(salt, channel_type, destination, code);
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Strip whitespace and dashes users tend to type ("123 456", "123-456").
pub fn normalize_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect()
}

/// Whether `row` holds a code that can still be confirmed at `now`.
pub fn is_pending(row: &notification_verification::Model, now: DateTime<Utc>) -> bool {
    row.code_hash.is_some()
        && row.attempts < MAX_ATTEMPTS
        && row.expires_at.is_some_and(|expires| expires > now)
}

async fn find(
    db: &DatabaseConnection,
    user_id: i64,
    channel_type: &str,
) -> Result<Option<notification_verification::Model>> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::ChannelType.eq(channel_type))
        .one(db)
        .await?)
}

/// Channels of `user_id` with a confirmable code at `now`.
pub async fn pending_channels(
    db: &DatabaseConnection,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<HashSet<String>> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .all(db)
        .await?
        .into_iter()
        .filter(|row| is_pending(row, now))
        .map(|row| row.channel_type)
        .collect())
}

/// Apply the send rate limit and record one send at `now`.
///
/// Returns [`AppError::TooManyRequests`] when the user must wait.
pub async fn reserve_send(
    db: &DatabaseConnection,
    user_id: i64,
    channel_type: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let Some(row) = find(db, user_id, channel_type).await? else {
        notification_verification::ActiveModel {
            user_id: Set(user_id),
            channel_type: Set(channel_type.to_string()),
            code_hash: Set(None),
            expires_at: Set(None),
            attempts: Set(0),
            last_sent_at: Set(Some(now)),
            window_started_at: Set(Some(now)),
            sends_in_window: Set(1),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await?;
        return Ok(());
    };

    if let Some(last) = row.last_sent_at {
        let wait = RESEND_INTERVAL_SECS - (now - last).num_seconds();
        if wait > 0 {
            return Err(AppError::TooManyRequests(format!(
                "Please wait {wait} seconds before requesting another verification code"
            )));
        }
    }

    let (window_start, sends) = match row.window_started_at {
        Some(start) if now - start < Duration::seconds(WINDOW_SECS) => (start, row.sends_in_window),
        _ => (now, 0),
    };
    if sends >= MAX_SENDS_PER_HOUR {
        let minutes = (window_start + Duration::seconds(WINDOW_SECS) - now).num_minutes() + 1;
        return Err(AppError::TooManyRequests(format!(
            "Too many verification codes requested; try again in {minutes} minutes"
        )));
    }

    // Optimistic: only succeeds if no concurrent request recorded a send meanwhile.
    let mut update = Entity::update_many()
        .col_expr(Column::LastSentAt, Expr::value(Some(now)))
        .col_expr(Column::WindowStartedAt, Expr::value(Some(window_start)))
        .col_expr(Column::SendsInWindow, Expr::value(sends + 1))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::Id.eq(row.id));
    update = match row.last_sent_at {
        Some(last) => update.filter(Column::LastSentAt.eq(last)),
        None => update.filter(Column::LastSentAt.is_null()),
    };
    if update.exec(db).await?.rows_affected == 0 {
        return Err(AppError::TooManyRequests(format!(
            "Please wait {RESEND_INTERVAL_SECS} seconds before requesting another verification code"
        )));
    }
    Ok(())
}

/// Store a fresh code (hash only) valid until `now + CODE_TTL_SECS`, resetting attempts.
///
/// Call after [`reserve_send`], which guarantees the row exists.
pub async fn store_code(
    db: &DatabaseConnection,
    user_id: i64,
    channel_type: &str,
    code_hash: String,
    now: DateTime<Utc>,
) -> Result<()> {
    Entity::update_many()
        .col_expr(Column::CodeHash, Expr::value(Some(code_hash)))
        .col_expr(
            Column::ExpiresAt,
            Expr::value(Some(now + Duration::seconds(CODE_TTL_SECS))),
        )
        .col_expr(Column::Attempts, Expr::value(0))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::UserId.eq(user_id))
        .filter(Column::ChannelType.eq(channel_type))
        .exec(db)
        .await?;
    Ok(())
}

/// Drop any pending code for this user and channel (rate-limit state is kept).
pub async fn clear_code(db: &DatabaseConnection, user_id: i64, channel_type: &str) -> Result<()> {
    Entity::update_many()
        .col_expr(Column::CodeHash, Expr::value(Option::<String>::None))
        .col_expr(
            Column::ExpiresAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(Column::Attempts, Expr::value(0))
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(Column::UserId.eq(user_id))
        .filter(Column::ChannelType.eq(channel_type))
        .exec(db)
        .await?;
    Ok(())
}

/// Result of checking a submitted code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Code matched; the pending code has been cleared.
    Verified,
    /// No code was requested (or it was already used/cleared).
    NoPendingCode,
    /// The code's lifetime has passed.
    Expired,
    /// All attempts for this code are used up.
    TooManyAttempts,
    /// Wrong code; `remaining` guesses left for this code.
    WrongCode { remaining: i32 },
}

/// Check `code` for (user, channel, destination), counting the attempt first.
pub async fn check_code(
    db: &DatabaseConnection,
    user_id: i64,
    channel_type: &str,
    destination: &str,
    code: &str,
    now: DateTime<Utc>,
) -> Result<CheckOutcome> {
    let Some(row) = find(db, user_id, channel_type).await? else {
        return Ok(CheckOutcome::NoPendingCode);
    };
    let Some(stored) = row.code_hash.as_deref() else {
        return Ok(CheckOutcome::NoPendingCode);
    };
    if row.expires_at.is_none_or(|expires| expires <= now) {
        return Ok(CheckOutcome::Expired);
    }
    if row.attempts >= MAX_ATTEMPTS {
        return Ok(CheckOutcome::TooManyAttempts);
    }

    // Count the attempt atomically so parallel guesses cannot exceed the limit.
    let counted = Entity::update_many()
        .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
        .filter(Column::Id.eq(row.id))
        .filter(Column::Attempts.lt(MAX_ATTEMPTS))
        .exec(db)
        .await?;
    if counted.rows_affected == 0 {
        return Ok(CheckOutcome::TooManyAttempts);
    }

    if code_matches(stored, channel_type, destination, code) {
        clear_code(db, user_id, channel_type).await?;
        return Ok(CheckOutcome::Verified);
    }
    Ok(CheckOutcome::WrongCode {
        remaining: Ord::max(MAX_ATTEMPTS - row.attempts - 1, 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_six_digits() {
        for _ in 0..200 {
            let code = generate_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn hash_is_salted_and_bound_to_destination() {
        let a = hash_code("email", "a@example.com", "123456");
        let b = hash_code("email", "a@example.com", "123456");
        assert_ne!(a, b, "salt must differ per hash");
        assert!(!a.contains("123456"));
        assert!(code_matches(&a, "email", "a@example.com", "123456"));
        assert!(!code_matches(&a, "email", "a@example.com", "123457"));
        assert!(!code_matches(&a, "email", "b@example.com", "123456"));
        assert!(!code_matches(&a, "telegram", "a@example.com", "123456"));
        assert!(!code_matches("garbage", "email", "a@example.com", "123456"));
    }

    #[test]
    fn normalize_strips_spaces_and_dashes() {
        assert_eq!(normalize_code(" 123 456 "), "123456");
        assert_eq!(normalize_code("123-456"), "123456");
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
