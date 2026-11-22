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
        auth_token: "at".into(),
        providertoken: "pt".into(),
        device_token: "dt".into(),
        uuid: "uu".into(),
        token: "t0".into(),
        accountid: Some(1),
        owner_userid: Some(7),
    }
}

/// A fake that records every session it was handed and serves canned pages.
/// `token_expired` makes the `account` probe reject (so the runner re-authenticates);
/// `login_rejects` makes the silent login reject too.
#[derive(Default)]
struct Fake {
    seen_tokens: Mutex<Vec<String>>,
    token_expired: bool,
    login_rejects: bool,
    groups: Vec<Value>,
    with_scores: bool,
    two_profiles: bool,
}

impl Source for Fake {
    async fn login(&self, s: &Session) -> Result<Session, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        if self.login_rejects {
            return Err(ClientError::Rejected(401));
        }
        Ok(Session {
            token: "t1".into(),
            ..s.clone()
        })
    }
    async fn profiles(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        if self.two_profiles {
            // Two non-shared profiles; the primary (usertype 1) is the owner.
            return Ok(vec![
                json!({"id": 8, "shortname": "REP", "isshared": 0, "usertype": 3}),
                json!({"id": 7, "shortname": "RAD", "isshared": 0, "usertype": 1}),
            ]);
        }
        Ok(vec![json!({"id": 7, "shortname": "ALPHA", "isshared": 0})])
    }
    async fn account(&self, s: &Session) -> Result<Value, ClientError> {
        self.seen_tokens.lock().unwrap().push(s.token.clone());
        if self.token_expired {
            return Err(ClientError::Rejected(401));
        }
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
        source: &str,
        _userid: i64,
        _lastupdate: i64,
        _offset: i64,
    ) -> Result<ScorePage, ClientError> {
        let items = match (self.with_scores, source) {
            (true, "healthindicator") => {
                vec![
                    json!({"scoreid": 55, "score_type": "vascular_age", "date": 1710000000, "score": 42.0}),
                ]
            }
            (true, "trajectory") => {
                vec![json!({"trajectoryid": 3, "type": "weight", "active": true})]
            }
            _ => vec![],
        };
        Ok(ScorePage {
            items,
            more: false,
            next_offset: None,
            raw: json!({}),
        })
    }
}

db_test!(a_rotated_token_is_saved_before_the_next_request, |f| {
    // The stored token is rejected, so the runner refreshes and must persist the rotated
    // session before going on.
    let fake = Fake {
        token_expired: true,
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

db_test!(a_valid_token_is_not_refreshed, |f| {
    // The probe succeeds, so the token is left alone — no refresh, no rotation.
    let fake = Fake {
        groups: vec![
            json!({"grpid": 9, "date": 1710000000, "measures": [{"value": 70, "type": 1, "unit": 0}]}),
        ],
        ..Default::default()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, seed_session());
    runner.pass().await.unwrap();
    assert_eq!(runner.session().token, "t0");
});

db_test!(a_rejected_session_is_a_rejection_not_a_crash, |f| {
    let fake = Fake {
        token_expired: true,
        login_rejects: true,
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

db_test!(scores_land_in_their_typed_tables, |f| {
    let fake = Fake {
        groups: vec![
            json!({"grpid": 1, "date": 1710000000, "measures": [{"value": 70, "type": 1, "unit": 0}]}),
        ],
        with_scores: true,
        ..Default::default()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, seed_session());
    runner.pass().await.unwrap();
    let score: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM indicator_score WHERE id = 55 AND person_id = 7 AND source = 'healthindicator'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(score, 1);
    let traj: i64 =
        sqlx::query_scalar("SELECT count(*) FROM trajectory WHERE id = 3 AND person_id = 7")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(traj, 1);
});

db_test!(the_primary_profile_is_owner_when_two_are_non_shared, |f| {
    let fake = Fake {
        two_profiles: true,
        ..Default::default()
    };
    // No owner in the seed, so the runner must pick it: the usertype-1 profile (id 7).
    let session = Session {
        owner_userid: None,
        ..seed_session()
    };
    let mut runner = Runner::new(f.pool.clone(), "owner".into(), 500, fake, session);
    runner.pass().await.unwrap();
    let owner_id: i64 = sqlx::query_scalar("SELECT id FROM person WHERE is_owner")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(owner_id, 7);
    let non_owner: bool = sqlx::query_scalar("SELECT NOT is_owner FROM person WHERE id = 8")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(non_owner);
});
