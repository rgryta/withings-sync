//! The session: the persistent device credentials the app uses to re-authenticate
//! silently (`auth?action=login`), plus the current session token. Stored verbatim in
//! `ingest_session.session`; its `Debug` is redacted so a stray log line cannot print a
//! credential.
//!
//! The device credentials (`auth_token`, `providertoken`, `device_token`, `uuid`) come
//! from the one interactive sign-in and do not expire or need a 2FA code — they are what
//! makes the ingester autonomous. `token` is the short-lived Session-Id the server hands
//! back and is re-minted by a silent login.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// Device credential: the account/device auth token (from the interactive sign-in).
    pub auth_token: String,
    /// Device credential: the provider token. May rotate on login; persist the latest.
    pub providertoken: String,
    /// Device credential: the device token (the sign-in's `token` field).
    pub device_token: String,
    /// The device uuid the sign-in was bound to.
    pub uuid: String,
    /// The active session token, carried as the `Session-Id` header. Re-minted by login.
    pub token: String,
    #[serde(default)]
    pub accountid: Option<i64>,
    #[serde(default)]
    pub owner_userid: Option<i64>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("auth_token", &"**********")
            .field("providertoken", &"**********")
            .field("device_token", &"**********")
            .field("uuid", &"**********")
            .field("token", &"**********")
            .field("accountid", &self.accountid)
            .field("owner_userid", &self.owner_userid)
            .finish()
    }
}
