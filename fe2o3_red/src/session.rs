//! Session store — O3db-backed session and conversation management.
//!
//! Sessions are keyed by user in O3db, supporting multi-user from
//! the start.  Each user has a list of session IDs and a per-user
//! config (default model, etc.).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::sync::{Arc, RwLock};
use std::marker::PhantomData;

use crate::protocol::{
    ChatMessage,
    Session,
    UserConfig,
    generate_session_id,
    sessions_key,
    session_key,
    user_config_key,
};


// ┌───────────────────────────────────────────────────────────────┐
// │ SessionStore                                                   │
// └───────────────────────────────────────────────────────────────┘

/// O3db-backed store for chat sessions and user configuration.
///
/// All operations are scoped by username — each user sees only their
/// own sessions.  This is the foundation for multi-user support.
///
/// Generic over the O3db database types, matching Steel's
/// `ServerContext` generics.
pub struct SessionStore<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
> {
    db:  Arc<RwLock<DB>>,
    uid: UID,
    _phantom_enc: PhantomData<ENC>,
    _phantom_kh:  PhantomData<KH>,
}

// SessionStore is Clone because Arc is Clone and UID: Clone.
impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
> Clone for SessionStore<UIDL, UID, ENC, KH, DB>
{
    fn clone(&self) -> Self {
        Self {
            db:  self.db.clone(),
            uid: self.uid.clone(),
            _phantom_enc: PhantomData,
            _phantom_kh:  PhantomData,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
>
    SessionStore<UIDL, UID, ENC, KH, DB>
{
    pub fn new(db: Arc<RwLock<DB>>, uid: UID) -> Self {
        Self {
            db,
            uid,
            _phantom_enc: PhantomData,
            _phantom_kh:  PhantomData,
        }
    }

    /// Create a new session for a user.
    pub fn create_session(
        &self,
        username:   &str,
        name:       &str,
        model:      &str,
    ) -> Outcome<Session> {
        let id = generate_session_id();
        let session = Session::new(id.clone(), name.to_string(), model.to_string());

        // Store the session.
        let session_dat = Dat::Map(session.to_datmap());
        {
            let db_w = match self.db.write() {
                Ok(v) => v,
                Err(_) => return Err(err!(
                    "SessionStore: database write lock poisoned.";
                    Lock, Poisoned)),
            };
            res!(db_w.insert(session_key(&id), session_dat, self.uid.clone(), None));
        }

        // Add to the user's session list.
        res!(self.add_to_user_sessions(username, &id));

        Ok(session)
    }

    /// List all sessions for a user (metadata only, no messages).
    pub fn list_sessions(&self, username: &str) -> Outcome<Vec<Session>> {
        let session_ids = res!(self.get_user_session_ids(username));
        let mut sessions = Vec::new();
        for id in session_ids {
            match self.get_session(&id) {
                Ok(s) => sessions.push(s),
                Err(_) => continue,  // skip missing sessions
            }
        }
        // Sort by created_at descending (newest first).
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(sessions)
    }

    /// Get a session by ID (includes messages).
    pub fn get_session(&self, session_id: &str) -> Outcome<Session> {
        let db_r = match self.db.read() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database read lock poisoned.";
                Lock, Poisoned)),
        };
        match db_r.get(&session_key(session_id), None) {
            Ok(Some((data, _))) => {
                match &data {
                    Dat::Map(m) => Session::from_datmap(m),
                    _ => Err(err!(
                        "SessionStore: session '{}' is not a map.", session_id;
                        Invalid, Data)),
                }
            }
            Ok(None) => Err(err!(
                "SessionStore: session '{}' not found.", session_id;
                NotFound, Missing)),
            Err(e) => Err(err!(e, "SessionStore: get session failed."; IO, Data, Read)),
        }
    }

    /// Append a message to a session's conversation history.
    pub fn append_message(
        &self,
        session_id: &str,
        message:   ChatMessage,
    ) -> Outcome<()> {
        let mut session = res!(self.get_session(session_id));
        session.messages.push(message);
        self.save_session(&session)
    }

    /// Save a session (full overwrite).
    pub fn save_session(&self, session: &Session) -> Outcome<()> {
        let session_dat = Dat::Map(session.to_datmap());
        let db_w = match self.db.write() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database write lock poisoned.";
                Lock, Poisoned)),
        };
        res!(db_w.insert(session_key(&session.id), session_dat, self.uid.clone(), None));
        Ok(())
    }

    /// Delete a session.
    pub fn delete_session(&self, username: &str, session_id: &str) -> Outcome<()> {
        // Remove from user's session list.
        res!(self.remove_from_user_sessions(username, session_id));

        // Delete the session record.
        let db_w = match self.db.write() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database write lock poisoned.";
                Lock, Poisoned)),
        };
        res!(db_w.delete(&session_key(session_id), self.uid.clone(), None));
        Ok(())
    }

    /// Rename a session.
    pub fn rename_session(&self, session_id: &str, new_name: &str) -> Outcome<()> {
        let mut session = res!(self.get_session(session_id));
        session.name = new_name.to_string();
        self.save_session(&session)
    }

    // ── User config ───────────────────────────────────────────

    /// Get or create user config.
    pub fn get_or_create_user_config(
        &self,
        username:       &str,
        default_model:  &str,
    ) -> Outcome<UserConfig> {
        // Try to read existing.
        let existing = {
            let db_r = match self.db.read() {
                Ok(v) => v,
                Err(_) => return Err(err!(
                    "SessionStore: database read lock poisoned.";
                    Lock, Poisoned)),
            };
            db_r.get(&user_config_key(username), None)
        };
        match existing {
            Ok(Some((data, _))) => {
                if let Dat::Map(m) = &data {
                    return UserConfig::from_datmap(m);
                }
            }
            _ => {}
        }
        // Create new.
        let config = UserConfig::new(username.to_string(), default_model.to_string());
        let config_dat = Dat::Map(config.to_datmap());
        {
            let db_w = match self.db.write() {
                Ok(v) => v,
                Err(_) => return Err(err!(
                    "SessionStore: database write lock poisoned.";
                    Lock, Poisoned)),
            };
            res!(db_w.insert(user_config_key(username), config_dat, self.uid.clone(), None));
        }
        Ok(config)
    }

    /// Update user config.
    pub fn save_user_config(&self, config: &UserConfig) -> Outcome<()> {
        let config_dat = Dat::Map(config.to_datmap());
        let db_w = match self.db.write() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database write lock poisoned.";
                Lock, Poisoned)),
        };
        res!(db_w.insert(user_config_key(&config.username), config_dat, self.uid.clone(), None));
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────

    fn get_user_session_ids(&self, username: &str) -> Outcome<Vec<String>> {
        let db_r = match self.db.read() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database read lock poisoned.";
                Lock, Poisoned)),
        };
        match db_r.get(&sessions_key(username), None) {
            Ok(Some((data, _))) => {
                match &data {
                    Dat::List(list) => {
                        let mut ids = Vec::new();
                        for item in list {
                            if let Dat::Str(s) = item {
                                ids.push(s.clone());
                            }
                        }
                        Ok(ids)
                    }
                    _ => Ok(Vec::new()),
                }
            }
            _ => Ok(Vec::new()),
        }
    }

    fn add_to_user_sessions(&self, username: &str, session_id: &str) -> Outcome<()> {
        let mut ids = res!(self.get_user_session_ids(username));
        ids.push(session_id.to_string());
        let list_dat = Dat::List(ids.into_iter().map(Dat::Str).collect());
        let db_w = match self.db.write() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database write lock poisoned.";
                Lock, Poisoned)),
        };
        res!(db_w.insert(sessions_key(username), list_dat, self.uid.clone(), None));
        Ok(())
    }

    fn remove_from_user_sessions(&self, username: &str, session_id: &str) -> Outcome<()> {
        let mut ids = res!(self.get_user_session_ids(username));
        ids.retain(|id| id != session_id);
        let list_dat = Dat::List(ids.into_iter().map(Dat::Str).collect());
        let db_w = match self.db.write() {
            Ok(v) => v,
            Err(_) => return Err(err!(
                "SessionStore: database write lock poisoned.";
                Lock, Poisoned)),
        };
        res!(db_w.insert(sessions_key(username), list_dat, self.uid.clone(), None));
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    // We test the protocol-level types and key generation here.
    // Full O3db integration tests require a running database,
    // which is tested in the Steel integration test suite.

    #[test]
    fn test_session_id_format() {
        let id = generate_session_id();
        assert!(!id.is_empty());
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_session_key_format() {
        let key = session_key("abc123");
        match key {
            Dat::Str(s) => assert_eq!(s, "red:session:abc123"),
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn test_sessions_key_format() {
        let key = sessions_key("jason");
        match key {
            Dat::Str(s) => assert_eq!(s, "red:jason:sessions"),
            _ => panic!("expected Str"),
        }
    }
}
