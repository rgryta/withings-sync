//! Client tests against a local server: the allow-list refuses unknown paths before
//! sending, refresh rotates the token, a non-zero status is a rejection not an HTTP
//! error, paging reports `more`, and no credential reaches `Debug`.

use std::net::SocketAddr;

use axum::{Router, extract::Request, response::Json, routing::any};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use withings_client::{Client, ClientError, Session};

fn session() -> Session {
    Session {
        sessionid: "SECRET-SESSIONID".into(),
        token: "SECRET-TOKEN".into(),
        accountid: Some(42),
        owner_userid: Some(7),
    }
}

/// A server that answers each cgi-bin path with a canned JSON body.
async fn serve() -> String {
    async fn handle(req: Request) -> Json<Value> {
        let path = req.uri().path().to_owned();
        let body = if path.ends_with("/session") {
            // Renew returns a fresh token, and here also a rotated sessionid.
            json!({"status": 0, "body": {"token": "NEW-TOKEN", "sessionid": "ROTATED-SID", "promoted": false}})
        } else if path.ends_with("/measure") {
            json!({"status": 0, "body": {
                "updatetime": 1710000000,
                "more": true,
                "offset": 200,
                "measuregrps": [{"grpid": 1, "measures": [{"value": 7234, "type": 1, "unit": -2}]}]
            }})
        } else if path.ends_with("/account") {
            // A rejected session: valid HTTP, non-zero Withings status.
            json!({"status": 401, "body": {}})
        } else {
            json!({"status": 0, "body": {}})
        };
        Json(body)
    }
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/cgi-bin/{*rest}", any(handle));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/cgi-bin/")
}

#[tokio::test]
async fn a_request_outside_the_allow_list_is_refused_before_sending() {
    // No server: an unknown score source must fail without a network call.
    let client = Client::with_base("http://127.0.0.1:1/cgi-bin/");
    let err = client
        .scores(&session(), "not-a-real-source", 7, 0, 0)
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::NotAllowed(_, _)), "{err:?}");
}

#[tokio::test]
async fn refresh_returns_the_rotated_tokens() {
    let base = serve().await;
    let client = Client::with_base(base);
    let rotated = client.refresh(&session()).await.unwrap();
    assert_eq!(rotated.token, "NEW-TOKEN");
    // A rotated sessionid in the response is taken, not dropped for the stale one.
    assert_eq!(rotated.sessionid, "ROTATED-SID");
    assert_eq!(rotated.owner_userid, Some(7));
}

#[tokio::test]
async fn a_rejected_session_is_rejected_not_http() {
    let base = serve().await;
    let client = Client::with_base(base);
    let err = client.account(&session()).await.unwrap_err();
    assert!(matches!(err, ClientError::Rejected(401)), "{err:?}");
}

#[tokio::test]
async fn paging_reports_more_and_offset() {
    let base = serve().await;
    let client = Client::with_base(base);
    let page = client.measures(&session(), 7, 1, 0, 0).await.unwrap();
    assert_eq!(page.groups.len(), 1);
    assert!(page.more);
    assert_eq!(page.next_offset, Some(200));
    assert_eq!(page.updatetime, 1710000000);
}

#[tokio::test]
async fn secrets_and_tokens_never_appear_in_debug() {
    let shown = format!("{:?}", session());
    assert!(!shown.contains("SECRET-SESSIONID"), "{shown}");
    assert!(!shown.contains("SECRET-TOKEN"), "{shown}");
    assert!(shown.contains("**********"));
}
