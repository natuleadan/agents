//! Valkey (Redis-compatible) ephemeral state store: locks, sliding-window rate limits,
//! presence, draft storage, and idempotency keys, plus an in-memory backend for hermetic tests.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use redis::AsyncCommands;
use tokio::sync::Mutex;

use crate::infra::error::ServerError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result of a sliding-window rate-limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub remaining: u64,
    pub reset_at_ms: u64,
}

/// Abstract ephemeral state store backed by Valkey in production or memory in tests.
pub trait StateBackend: Send + Sync + 'static {
    fn acquire_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, ServerError>>;

    fn release_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<bool, ServerError>>;

    fn check_rate_limit<'a>(
        &'a self,
        bucket: &'a str,
        max_requests: u64,
        window: Duration,
    ) -> BoxFuture<'a, Result<RateLimitDecision, ServerError>>;

    fn get_idempotency<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>>;

    fn put_idempotency<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn set_presence<'a>(
        &'a self,
        actor_id: &'a str,
        status: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn get_presence<'a>(
        &'a self,
        actor_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>>;

    fn save_draft<'a>(
        &'a self,
        session_id: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn load_draft<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>>;

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>>;
}

/// Production Valkey backend using `redis::aio::ConnectionManager`.
#[derive(Clone)]
pub struct ValkeyStateBackend {
    conn: redis::aio::ConnectionManager,
}

impl ValkeyStateBackend {
    /// Connects to Valkey using the configured Redis-compatible URL.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Internal`] when the URL or initial connection fails.
    pub async fn connect(url: &str) -> Result<Arc<Self>, ServerError> {
        let client = redis::Client::open(url)
            .map_err(|err| ServerError::Internal(format!("invalid VALKEY_URL: {err}")))?;
        let conn = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|err| ServerError::Internal(format!("valkey connection failed: {err}")))?;
        Ok(Arc::new(Self { conn }))
    }
}

impl StateBackend for ValkeyStateBackend {
    fn acquire_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let opts = redis::SetOptions::default()
                .conditional_set(redis::ExistenceCheck::NX)
                .with_expiration(redis::SetExpiry::PX(ttl.as_millis() as u64));
            let res: Option<String> = conn
                .set_options(format!("lock:{key}"), owner, opts)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey lock error: {e}")))?;
            Ok(res.is_some())
        })
    }

    fn release_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let script = redis::Script::new(
                "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end",
            );
            let removed: i32 = script
                .key(format!("lock:{key}"))
                .arg(owner)
                .invoke_async(&mut conn)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey unlock error: {e}")))?;
            Ok(removed > 0)
        })
    }

    fn check_rate_limit<'a>(
        &'a self,
        bucket: &'a str,
        max_requests: u64,
        window: Duration,
    ) -> BoxFuture<'a, Result<RateLimitDecision, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let now = now_ms();
            let window_ms = window.as_millis().max(1) as u64;
            let cutoff = now.saturating_sub(window_ms);
            let redis_key = format!("rl:{bucket}");
            let member = format!("{now}-{}", uuid::Uuid::new_v4());
            let script = redis::Script::new(
                r#"
                redis.call('ZREMRANGEBYSCORE', KEYS[1], 0, ARGV[1])
                local count = redis.call('ZCARD', KEYS[1])
                if tonumber(count) >= tonumber(ARGV[3]) then
                    return {0, 0}
                end
                redis.call('ZADD', KEYS[1], ARGV[2], ARGV[4])
                redis.call('PEXPIRE', KEYS[1], ARGV[5])
                return {1, tonumber(ARGV[3]) - tonumber(count) - 1}
                "#,
            );
            let (allowed, remaining): (i64, i64) = script
                .key(redis_key)
                .arg(cutoff)
                .arg(now)
                .arg(max_requests)
                .arg(member)
                .arg(window_ms)
                .invoke_async(&mut conn)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey rate-limit error: {e}")))?;

            Ok(RateLimitDecision {
                allowed: allowed == 1,
                remaining: remaining.max(0) as u64,
                reset_at_ms: now + window_ms,
            })
        })
    }

    fn get_idempotency<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            conn.get(format!("idem:{key}"))
                .await
                .map_err(|e| ServerError::Internal(format!("valkey get idempotency error: {e}")))
        })
    }

    fn put_idempotency<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let secs = ttl.as_secs().max(1);
            let _: () = conn
                .set_ex(format!("idem:{key}"), value, secs)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey put idempotency error: {e}")))?;
            Ok(())
        })
    }

    fn set_presence<'a>(
        &'a self,
        actor_id: &'a str,
        status: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let secs = ttl.as_secs().max(1);
            let _: () = conn
                .set_ex(format!("presence:{actor_id}"), status, secs)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey presence error: {e}")))?;
            Ok(())
        })
    }

    fn get_presence<'a>(
        &'a self,
        actor_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            conn.get(format!("presence:{actor_id}"))
                .await
                .map_err(|e| ServerError::Internal(format!("valkey get presence error: {e}")))
        })
    }

    fn save_draft<'a>(
        &'a self,
        session_id: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let _: () = conn
                .set_ex(format!("draft:{session_id}"), content, 604_800)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey save draft error: {e}")))?;
            Ok(())
        })
    }

    fn load_draft<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            conn.get(format!("draft:{session_id}"))
                .await
                .map_err(|e| ServerError::Internal(format!("valkey load draft error: {e}")))
        })
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let _: String = redis::cmd("PING")
                .query_async(&mut conn)
                .await
                .map_err(|e| ServerError::Internal(format!("valkey ping failed: {e}")))?;
            Ok(())
        })
    }
}

#[derive(Default)]
struct MemoryStateInner {
    locks: HashMap<String, (String, u64)>,
    windows: HashMap<String, VecDeque<u64>>,
    idempotency: HashMap<String, (String, u64)>,
    presence: HashMap<String, (String, u64)>,
    drafts: HashMap<String, String>,
}

/// Deterministic in-memory state store used when Valkey is omitted (dev/hermetic tests).
#[derive(Clone, Default)]
pub struct InMemoryStateBackend {
    inner: Arc<Mutex<MemoryStateInner>>,
}

impl InMemoryStateBackend {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl StateBackend for InMemoryStateBackend {
    fn acquire_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let now = now_ms();
            let mut inner = self.inner.lock().await;
            if let Some((current_owner, exp)) = inner.locks.get(key)
                && *exp > now
                && current_owner != owner
            {
                return Ok(false);
            }
            inner.locks.insert(
                key.to_owned(),
                (owner.to_owned(), now + ttl.as_millis() as u64),
            );
            Ok(true)
        })
    }

    fn release_lock<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            if let Some((current_owner, _)) = inner.locks.get(key)
                && current_owner == owner
            {
                inner.locks.remove(key);
                return Ok(true);
            }
            Ok(false)
        })
    }

    fn check_rate_limit<'a>(
        &'a self,
        bucket: &'a str,
        max_requests: u64,
        window: Duration,
    ) -> BoxFuture<'a, Result<RateLimitDecision, ServerError>> {
        Box::pin(async move {
            let now = now_ms();
            let window_ms = window.as_millis().max(1) as u64;
            let cutoff = now.saturating_sub(window_ms);
            let mut inner = self.inner.lock().await;
            let entries = inner.windows.entry(bucket.to_owned()).or_default();
            while entries.front().is_some_and(|ts| *ts <= cutoff) {
                entries.pop_front();
            }
            if entries.len() as u64 >= max_requests {
                return Ok(RateLimitDecision {
                    allowed: false,
                    remaining: 0,
                    reset_at_ms: now + window_ms,
                });
            }
            entries.push_back(now);
            let remaining = max_requests.saturating_sub(entries.len() as u64);
            Ok(RateLimitDecision {
                allowed: true,
                remaining,
                reset_at_ms: now + window_ms,
            })
        })
    }

    fn get_idempotency<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move {
            let now = now_ms();
            let mut inner = self.inner.lock().await;
            if let Some((val, exp)) = inner.idempotency.get(key).cloned() {
                if exp > now {
                    return Ok(Some(val));
                }
                inner.idempotency.remove(key);
            }
            Ok(None)
        })
    }

    fn put_idempotency<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let exp = now_ms() + ttl.as_millis() as u64;
            let mut inner = self.inner.lock().await;
            inner
                .idempotency
                .insert(key.to_owned(), (value.to_owned(), exp));
            Ok(())
        })
    }

    fn set_presence<'a>(
        &'a self,
        actor_id: &'a str,
        status: &'a str,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let exp = now_ms() + ttl.as_millis() as u64;
            let mut inner = self.inner.lock().await;
            inner
                .presence
                .insert(actor_id.to_owned(), (status.to_owned(), exp));
            Ok(())
        })
    }

    fn get_presence<'a>(
        &'a self,
        actor_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move {
            let now = now_ms();
            let mut inner = self.inner.lock().await;
            if let Some((status, exp)) = inner.presence.get(actor_id).cloned() {
                if exp > now {
                    return Ok(Some(status));
                }
                inner.presence.remove(actor_id);
            }
            Ok(None)
        })
    }

    fn save_draft<'a>(
        &'a self,
        session_id: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner
                .lock()
                .await
                .drafts
                .insert(session_id.to_owned(), content.to_owned());
            Ok(())
        })
    }

    fn load_draft<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ServerError>> {
        Box::pin(async move { Ok(self.inner.lock().await.drafts.get(session_id).cloned()) })
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async { Ok(()) })
    }
}

#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Concrete DI wrapper around [`StateBackend`].
#[derive(Clone)]
pub struct StateStore(pub Arc<dyn StateBackend>);

impl Default for StateStore {
    fn default() -> Self {
        Self(InMemoryStateBackend::new())
    }
}
