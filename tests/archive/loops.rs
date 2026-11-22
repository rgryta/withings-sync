//! Runner tests against a fake Source and the shared test database: the rotated token is
//! saved before the next request, a rejection is a rejection (not a crash), a deleted
//! group is marked, and backfill respects the budget and finishes.

use std::sync::Mutex;

use serde_json::{Value, json};
use withings_client::{ClientError, MeasPage, ScorePage, Session};
use withings_ingest::runner::{self, RunError, Runner};
use withings_ingest::source::Source;

use crate::support::db_test;

fn seed_session() -> Session {
    Session {
        sessionid: "sid".into(),
        token: "t0".into(),
        accountid: Some(1),
        owner_userid: Some(7),
    }
}

/// A fake that records every session it was handed and serves canned pages. `reject`
/// makes `refresh` reject; `pages` are returned by `measures` in order, then empty.
#[derive(Default)]
struct Fake {
    seen_tokens: Mutex<Vec<String>>,
    reject: bool,
    groups: Vec<Value>,
}

impl Source for Fake {
    async fn refresh(&self, s: &Session) -> Result<Session, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        if self.reject {
            return Err(ClientError::Rejected(401));
        }
        Ok(Session {
            token: "t1".into(),
            ..s.clone()
        })
    }
    async fn profiles(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        Ok(vec![json!({"id": 7, "shortname": "ALPHA", "isshared": 0})])
    }
    async fn account(&self, _s: &Session) -> Result<Value, ClientError> {
        Ok(json!({}))
    }
    async fn devices(&self, _s: &Session) -> Result<Vec<Value>, ClientError> {
        Ok(vec![])
    }
    async fn measures(
        &self,
        s: &Session,
        _userid: i64,
        _category: i64,
        _lastupdate: i64,
        offset: i64,
    ) -> Result<MeasPage, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        let groups = if offset == 0 {
            self.groups.clone()
        } else {
            vec![]
        };
        Ok(MeasPage {
            groups,
            updatetime: 1710000000,
            more: false,
            next_offset: None,
            raw: json!({}),
        })
    }
    async fn scores(
        &self,
        _s: &Session,
        _source: &str,
        _userid: i64,
        _lastupdate: i64,
        _offset: i64,
    ) -> Result<ScorePage, ClientError> {
        Ok(ScorePage {
            items: vec![],
            more: false,
            next_offset: None,
            raw: json!({}),
        })
    }
}

db_test!(a_rotated_token_is_saved_before_the_next_request, |f| {
    let fake = Fake {
        groups: vec![
            json!({"grpid": 1, "date": 1710000000, "measures": [{"value": 70, "type": 1, "unit": 0}]}),
        ],
        ..Default::default()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, seed_session());
    runner.pass().await.unwrap();
    // The session in the DB is the rotated one (token t1), persisted by authenticate
    // before profiles/measures ran.
    let stored: Value =
        sqlx::query_scalar("SELECT session FROM ingest_session WHERE account_id = 'owner'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(stored.get("token").and_then(|v| v.as_str()), Some("t1"));
    // And the runner's own session advanced.
    assert_eq!(runner.session().token, "t1");
});

db_test!(a_rejected_session_is_a_rejection_not_a_crash, |f| {
    let fake = Fake {
        reject: true,
        ..Default::default()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, seed_session());
    let err = runner.pass().await.unwrap_err();
    assert!(matches!(err, RunError::Rejected(401)), "{err:?}");
});

db_test!(a_deleted_group_is_marked_deleted, |f| {
    let fake = Fake {
        groups: vec![json!({"grpid": 2, "date": 1710000000, "measures": [], "deleted": 1})],
        ..Default::default()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, seed_session());
    runner.pass().await.unwrap();
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM measure_group WHERE grpid = 2")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(deleted);
});

db_test!(backfill_finishes_and_respects_the_budget, |f| {
    let fake = Fake {
        groups: vec![
            json!({"grpid": 3, "date": 1710000000, "measures": [{"value": 70, "type": 1, "unit": 0}]}),
        ],
        ..Default::default()
    };
    // Budget of 1: the first step fetches, finds no more pages, marks done.
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 1, fake, seed_session());
    assert!(runner.backfill_step().await.unwrap(), "one page, so done");
    let present: bool =
        sqlx::query_scalar("SELECT exists(SELECT 1 FROM measure_group WHERE grpid = 3)")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(present);
    let done: bool = sqlx::query_scalar("SELECT backfill_done FROM sync_state WHERE loop = $1")
        .bind(runner::LOOP)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(done);
});

db_test!(backfill_stops_when_the_budget_is_spent, |f| {
    let fake = Fake::default();
    // Budget 0: no request may be spent, so the step returns false without finishing.
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 0, fake, seed_session());
    assert!(
        !runner.backfill_step().await.unwrap(),
        "budget 0 blocks the step"
    );
    let done: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT backfill_done FROM sync_state WHERE loop = $1",
    )
    .bind(runner::LOOP)
    .fetch_optional(&f.pool)
    .await
    .unwrap()
    .flatten()
    .unwrap_or(false);
    assert!(!done);
});
