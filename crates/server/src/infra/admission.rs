//! Per-scope admission control and backpressure guards (`Session`, `Global`, `Readonly`).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use agents_protocol::ExecutionScope;

use crate::infra::error::ServerError;

/// RAII lease releasing an admitted slot when dropped (prevents counter leaks on early return).
#[derive(Debug)]
pub struct AdmissionPermit {
    scope: ExecutionScope,
    session_id: Option<String>,
    controller: Arc<AdmissionController>,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        self.controller
            .release(self.scope, self.session_id.as_deref());
    }
}

/// Bounds concurrent operations per [`ExecutionScope`] and fails fast with [`ServerError::Overloaded`].
#[derive(Debug)]
pub struct AdmissionController {
    max_global: usize,
    max_per_session: usize,
    active_global: AtomicUsize,
    active_sessions: Mutex<HashMap<String, usize>>,
}

impl AdmissionController {
    #[must_use]
    pub fn new(max_global: usize, max_per_session: usize) -> Arc<Self> {
        Arc::new(Self {
            max_global: max_global.max(1),
            max_per_session: max_per_session.max(1),
            active_global: AtomicUsize::new(0),
            active_sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Acquires an execution slot for the requested scope or returns [`ServerError::Overloaded`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Overloaded`] when global or per-session concurrency limits are saturated.
    pub fn try_acquire(
        self: &Arc<Self>,
        scope: ExecutionScope,
        session_id: Option<&str>,
    ) -> Result<AdmissionPermit, ServerError> {
        match scope {
            ExecutionScope::Readonly => Ok(AdmissionPermit {
                scope,
                session_id: None,
                controller: Arc::clone(self),
            }),
            ExecutionScope::Global => {
                self.increment_global()?;
                Ok(AdmissionPermit {
                    scope,
                    session_id: None,
                    controller: Arc::clone(self),
                })
            }
            ExecutionScope::Session => {
                let sid = session_id.ok_or_else(|| {
                    ServerError::Validation(
                        "session_id required for Session execution scope".into(),
                    )
                })?;
                self.increment_global()?;
                let mut map = self.active_sessions.lock().map_err(|_| {
                    self.active_global.fetch_sub(1, Ordering::SeqCst);
                    ServerError::Internal("admission lock poisoned".into())
                })?;
                let count = map.get(sid).copied().unwrap_or(0);
                if count >= self.max_per_session {
                    drop(map);
                    self.active_global.fetch_sub(1, Ordering::SeqCst);
                    return Err(ServerError::Overloaded(format!(
                        "session `{sid}` admission capacity ({}) saturated",
                        self.max_per_session
                    )));
                }
                map.insert(sid.to_owned(), count + 1);
                Ok(AdmissionPermit {
                    scope,
                    session_id: Some(sid.to_owned()),
                    controller: Arc::clone(self),
                })
            }
        }
    }

    #[must_use]
    pub fn active_global(&self) -> usize {
        self.active_global.load(Ordering::SeqCst)
    }

    fn increment_global(&self) -> Result<(), ServerError> {
        let mut current = self.active_global.load(Ordering::SeqCst);
        loop {
            if current >= self.max_global {
                return Err(ServerError::Overloaded(format!(
                    "global admission capacity ({}) saturated",
                    self.max_global
                )));
            }
            match self.active_global.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    fn release(&self, scope: ExecutionScope, session_id: Option<&str>) {
        match scope {
            ExecutionScope::Readonly => {}
            ExecutionScope::Global => {
                self.active_global.fetch_sub(1, Ordering::SeqCst);
            }
            ExecutionScope::Session => {
                if let Some(sid) = session_id
                    && let Ok(mut map) = self.active_sessions.lock()
                    && let Some(count) = map.get_mut(sid)
                {
                    if *count <= 1 {
                        map.remove(sid);
                    } else {
                        *count -= 1;
                    }
                }
                self.active_global.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}
