//! The ingest loops. A pass refreshes the token (and persists it before any further
//! request), syncs every profile's measurements from the watermark forward, and records
//! the scores. Backfill walks the whole history once under a daily request budget.
//!
//! The client never logs in. A rejected session is a `RunError::Rejected` the caller
//! records and waits on; the loop never exits on it.

use serde_json::{Value, json};
use sqlx::postgres::PgPool;
use withings_client::{ClientError, Session};
use withings_core::models::Person;
use withings_core::store::{measures, people, raw, scores, session as session_store, sync};

use crate::normalize;
use crate::source::Source;

/// The loop name under which the poll watermark and backfill flag live.
pub const LOOP: &str = "sync";

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("the Withings session was rejected (status {0})")]
    Rejected(i64),
    #[error("rate limited")]
    Throttled,
    #[error("client error: {0}")]
    Client(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<ClientError> for RunError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Rejected(c) => RunError::Rejected(c),
            ClientError::Throttled => RunError::Throttled,
            other => RunError::Client(other.to_string()),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct PassReport {
    pub groups: u64,
    pub revised: u64,
    pub deleted: u64,
    pub scores: u64,
}

pub struct Runner<S> {
    pool: PgPool,
    account: String,
    budget: i64,
    source: S,
    session: Session,
    backfill_offset: i64,
}

impl<S: Source> Runner<S> {
    pub fn new(pool: PgPool, account: String, budget: i64, source: S, session: Session) -> Self {
        Self {
            pool,
            account,
            budget,
            source,
            session,
            backfill_offset: 0,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Refresh the token and persist the rotated session **before** issuing any further
    /// request, so a crash right after the refresh still finds the new token saved.
    async fn authenticate(&mut self) -> Result<(), RunError> {
        let rotated = self.source.refresh(&self.session).await?;
        let mut conn = self.pool.acquire().await?;
        session_store::save(&mut conn, &self.account, &json!(rotated)).await?;
        self.session = rotated;
        Ok(())
    }

    /// Fetch every profile, mark the owner, upsert them, and return their userids.
    async fn sync_profiles(&mut self) -> Result<Vec<i64>, RunError> {
        let users = self.source.profiles(&self.session).await?;
        let mut parsed: Vec<Person> = Vec::new();
        for u in &users {
            if let Ok(p) = normalize::person(u) {
                parsed.push(p);
            }
        }
        // The owner is the session's userid when known. Otherwise, among the non-shared
        // profiles, the account holder's own is the primary one (Withings `usertype` 1,
        // and the only one carrying the account email); fall back to the sole non-shared
        // profile if that signal is absent.
        let owner = self.session.owner_userid.or_else(|| {
            let non_shared: Vec<&Person> = parsed.iter().filter(|p| !p.shared).collect();
            non_shared
                .iter()
                .find(|p| p.raw.get("usertype").and_then(serde_json::Value::as_i64) == Some(1))
                .map(|p| p.id)
                .or_else(|| (non_shared.len() == 1).then(|| non_shared[0].id))
        });
        let mut conn = self.pool.acquire().await?;
        let mut ids = Vec::new();
        for mut p in parsed {
            p.is_owner = Some(p.id) == owner;
            people::upsert(&mut conn, &p).await?;
            ids.push(p.id);
        }
        if self.session.owner_userid.is_none() && owner.is_some() {
            self.session.owner_userid = owner;
        }
        Ok(ids)
    }

    /// Sync one profile's measurement groups from `lastupdate` forward, paging by offset.
    /// Returns (max updatetime, groups, revised, deleted).
    async fn sync_user(
        &mut self,
        userid: i64,
        lastupdate: i64,
    ) -> Result<(i64, u64, u64, u64), RunError> {
        let mut offset = 0;
        let mut max_update = lastupdate;
        let (mut groups, mut revised, mut deleted) = (0u64, 0u64, 0u64);
        loop {
            let page = self
                .source
                .measures(&self.session, userid, 1, lastupdate, offset)
                .await?;
            max_update = max_update.max(page.updatetime);
            let mut conn = self.pool.acquire().await?;
            raw::upsert(
                &mut conn,
                "getmeas",
                &format!("{userid}:{lastupdate}:{offset}"),
                &page.raw,
            )
            .await?;
            for g in &page.groups {
                let mut mg = match normalize::group(g) {
                    Ok(mg) => mg,
                    Err(_) => continue,
                };
                mg.person_id = Some(userid);
                if normalize::group_is_deleted(g) {
                    measures::upsert_group(&mut conn, &mg).await?;
                    measures::mark_deleted(&mut conn, mg.grpid).await?;
                    deleted += 1;
                } else {
                    match measures::upsert_group(&mut conn, &mg).await? {
                        withings_core::models::GroupChange::New => groups += 1,
                        withings_core::models::GroupChange::Revised => revised += 1,
                        withings_core::models::GroupChange::Unchanged => {}
                    }
                }
            }
            if page.more && page.next_offset.map(|o| o != offset).unwrap_or(false) {
                offset = page.next_offset.unwrap();
            } else {
                break;
            }
        }
        Ok((max_update, groups, revised, deleted))
    }

    /// Sync the derived scores for a profile.
    async fn sync_scores(&mut self, userid: i64, lastupdate: i64) -> Result<u64, RunError> {
        let mut count = 0u64;
        for source in ["healthindicator", "vitality", "trajectory"] {
            let mut offset = 0;
            loop {
                let page = self
                    .source
                    .scores(&self.session, source, userid, lastupdate, offset)
                    .await?;
                let mut conn = self.pool.acquire().await?;
                raw::upsert(
                    &mut conn,
                    source,
                    &format!("{userid}:{lastupdate}:{offset}"),
                    &page.raw,
                )
                .await?;
                for item in &page.items {
                    if source == "trajectory" {
                        if let Ok(mut t) = normalize::trajectory(item) {
                            t.person_id = Some(userid);
                            scores::upsert_trajectory(&mut conn, &t).await?;
                            count += 1;
                        }
                    } else if let Ok(mut sc) = normalize::score(source, item) {
                        sc.person_id = Some(userid);
                        scores::upsert_score(&mut conn, &sc).await?;
                        count += 1;
                    }
                }
                if page.more && page.next_offset.map(|o| o != offset).unwrap_or(false) {
                    offset = page.next_offset.unwrap();
                } else {
                    break;
                }
            }
        }
        Ok(count)
    }

    /// One poll pass: refresh, sync every profile from the watermark, record scores.
    pub async fn pass(&mut self) -> Result<PassReport, RunError> {
        self.authenticate().await?;
        let watermark = sync::watermark(&self.pool, LOOP).await?.unwrap_or(0);
        let userids = self.sync_profiles().await?;
        let mut report = PassReport::default();
        let mut newest = watermark;
        for userid in userids {
            let (max_update, groups, revised, deleted) = self.sync_user(userid, watermark).await?;
            report.groups += groups;
            report.revised += revised;
            report.deleted += deleted;
            report.scores += self.sync_scores(userid, watermark).await?;
            newest = newest.max(max_update);
        }
        if newest > watermark {
            sync::set_watermark(&self.pool, LOOP, newest).await?;
        }
        sync::record_ok(&self.pool, LOOP).await?;
        Ok(report)
    }

    /// One backfill step: spend one request's budget to page the owner's full history from
    /// the start. Returns true when the history is exhausted (backfill done). Returns
    /// false — without fetching — when the daily budget is spent.
    pub async fn backfill_step(&mut self) -> Result<bool, RunError> {
        if sync::is_backfill_done(&self.pool, LOOP).await? {
            return Ok(true);
        }
        if !sync::take_budget(&self.pool, LOOP, 1, self.budget).await? {
            return Ok(false);
        }
        self.authenticate().await?;
        let owner = match self.session.owner_userid {
            Some(id) => id,
            None => {
                // Learn the owner first.
                let ids = self.sync_profiles().await?;
                match self.session.owner_userid.or_else(|| ids.first().copied()) {
                    Some(id) => id,
                    None => return Ok(true),
                }
            }
        };
        let page = self
            .source
            .measures(&self.session, owner, 1, 0, self.backfill_offset)
            .await?;
        let mut conn = self.pool.acquire().await?;
        raw::upsert(
            &mut conn,
            "getmeas",
            &format!("backfill:{owner}:{}", self.backfill_offset),
            &page.raw,
        )
        .await?;
        people::ensure(&mut conn, owner, true).await?;
        for g in &page.groups {
            if let Ok(mut mg) = normalize::group(g) {
                mg.person_id = Some(owner);
                measures::upsert_group(&mut conn, &mg).await?;
                if normalize::group_is_deleted(g) {
                    measures::mark_deleted(&mut conn, mg.grpid).await?;
                }
            }
        }
        if page.more
            && page
                .next_offset
                .map(|o| o != self.backfill_offset)
                .unwrap_or(false)
        {
            self.backfill_offset = page.next_offset.unwrap();
            Ok(false)
        } else {
            sync::mark_backfill_done(&self.pool, LOOP).await?;
            Ok(true)
        }
    }
}

/// Load the session from the DB, or seed it from a file on first start. `None` means no
/// session is available and the caller must wait for a human to seed one.
pub async fn load_session(
    pool: &PgPool,
    account: &str,
    seed_path: Option<&str>,
) -> Result<Option<Session>, RunError> {
    if let Some(value) = session_store::load(pool, account).await? {
        return Ok(Some(serde_json::from_value(value).map_err(|e| {
            RunError::Client(format!("stored session is malformed: {e}"))
        })?));
    }
    if let Some(path) = seed_path
        && let Ok(text) = std::fs::read_to_string(path)
        && let Ok(value) = serde_json::from_str::<Value>(&text)
        && let Ok(seed) = serde_json::from_value::<Session>(value.clone())
    {
        let mut conn = pool.acquire().await?;
        session_store::save(&mut conn, account, &value).await?;
        return Ok(Some(seed));
    }
    Ok(None)
}
