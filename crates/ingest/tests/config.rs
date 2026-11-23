//! Config defaults. This test owns the process (its own test binary), so setting the
//! required vars and reading the rest is race-free. One test, so the env is touched from
//! one place only.

use std::time::Duration;

use withings_ingest::config::IngestSettings;

#[test]
fn defaults_and_a_missing_dsn() {
    // SAFETY: one test in its own process; nothing else touches the environment.
    unsafe {
        std::env::set_var("WITHINGS_DSN", "postgresql://u:p@h/withings");
        std::env::set_var("WITHINGS_ACCOUNT", "owner");
        std::env::remove_var("WITHINGS_POLL_INTERVAL");
        std::env::remove_var("WITHINGS_AUTH_RETRY");
        std::env::remove_var("WITHINGS_BACKFILL_BUDGET");
        std::env::remove_var("WITHINGS_SESSION_FROM");
    }
    let s = IngestSettings::from_env().unwrap();
    assert_eq!(s.account, "owner");
    assert_eq!(s.poll_interval, Duration::from_secs(20 * 60));
    assert_eq!(s.auth_retry, Duration::from_secs(6 * 60 * 60));
    assert_eq!(s.backfill_budget, 500);
    assert_eq!(s.session_from, None);
    // The DSN is a Secret; its Debug must not print the password.
    assert!(!format!("{:?}", s.dsn).contains("p@h"));

    unsafe {
        std::env::remove_var("WITHINGS_DSN");
    }
    assert!(IngestSettings::from_env().is_err());
}
