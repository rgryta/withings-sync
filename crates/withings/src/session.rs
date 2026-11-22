//! The session: the persistent `sessionid` and the short-lived `token`, plus the ids the
//! ingester needs. Stored verbatim in `ingest_session.session`; its `Debug` is redacted
//! so a stray log line cannot print the credential.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// The persistent credential. Survives restarts; only a human login replaces it.
    pub sessionid: String,
    /// The short-lived access token, carried as the `Session-Id` header and re-minted by
    /// `refresh`.
    pub token: String,
    /// The Withings account id, learned at login.
    #[serde(default)]
    pub accountid: Option<i64>,
    /// The owner's userid, so the ingester knows which profile is `is_owner`.
    #[serde(default)]
    pub owner_userid: Option<i64>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("sessionid", &"**********")
            .field("token", &"**********")
            .field("accountid", &self.accountid)
            .field("owner_userid", &self.owner_userid)
            .finish()
    }
}
