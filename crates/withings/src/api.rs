//! The Withings private-API client. Every request goes through one function that refuses
//! anything not in [`ALLOWED`]: the read-only guarantee is a code fact, not a promise.
//!
//! Auth is a `Session-Id` header carrying the short-lived token; `refresh` re-mints it
//! from the persistent `sessionid`. The client never logs in — that is a human's job.

use serde_json::Value;

use crate::error::ClientError;
use crate::session::Session;

/// The base the Withings app talks to.
pub const DEFAULT_BASE: &str = "https://scalews.withings.net/cgi-bin/";

/// Constant request identifiers the app sends. `appliver` is the app's version code and
/// may need bumping if Withings ever rejects an old one; it is not a secret.
const APPNAME: &str = "wiscaleNG";
const APPLIVER: &str = "202611091";
const APPPFM: &str = "android";
const CALLCTX: &str = "foreground";

/// Every request this client may issue: method and path under the cgi-bin base. A read
/// call, or the session refresh. Nothing here writes to the account.
pub const ALLOWED: &[(&str, &str)] = &[
    ("POST", "session"),            // action=renew
    ("POST", "account"),            // action=get / getuserslist
    ("POST", "association"),        // action=getbyaccountid (devices)
    ("GET", "v2/user"),             // action=get (one profile, enriched)
    ("POST", "measure"),            // action=getmeas
    ("POST", "v2/healthindicator"), // action=list
    ("POST", "v2/vitality"),        // action=list
    ("POST", "v2/trajectory"),      // action=list
];

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
}

/// Withings spells `more` as a bool on some endpoints and an integer on others; both
/// mean the same "there is another page".
fn truthy_more(body: &Value) -> bool {
    match body.get("more") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().map(|v| v != 0).unwrap_or(false),
        _ => false,
    }
}

/// A page of measurement groups.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasPage {
    pub groups: Vec<Value>,
    pub updatetime: i64,
    pub more: bool,
    pub next_offset: Option<i64>,
    pub raw: Value,
}

/// A page of scores (healthindicator / vitality / trajectory share this shape).
#[derive(Debug, Clone, PartialEq)]
pub struct ScorePage {
    pub items: Vec<Value>,
    pub more: bool,
    pub next_offset: Option<i64>,
    pub raw: Value,
}

impl Client {
    pub fn new() -> Self {
        Self::with_base(DEFAULT_BASE)
    }

    /// A client against a given base, for tests.
    pub fn with_base(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("okhttp/5.5.0")
            .build()
            .expect("a default reqwest client builds");
        let mut base = base.into();
        if !base.ends_with('/') {
            base.push('/');
        }
        Self { http, base }
    }

    /// POST a form to `path?action=...`, carrying the session token when given, and
    /// return the `body` object after checking `status == 0`.
    async fn call(
        &self,
        session: Option<&Session>,
        method: &'static str,
        path: &str,
        action: &str,
        form: &[(&str, String)],
    ) -> Result<Value, ClientError> {
        if !ALLOWED.iter().any(|(m, p)| *m == method && *p == path) {
            return Err(ClientError::NotAllowed(method, path.to_owned()));
        }
        let url = format!("{}{}", self.base, path);
        let query = [
            ("action", action),
            ("appname", APPNAME),
            ("appliver", APPLIVER),
            ("apppfm", APPPFM),
            ("callctx", CALLCTX),
        ];
        let mut req = match method {
            "GET" => self.http.get(&url).query(&query),
            _ => self.http.post(&url).query(&query).form(form),
        };
        if let Some(s) = session {
            req = req.header("Session-Id", &s.token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(ClientError::Throttled);
        }
        if !status.is_success() {
            return Err(ClientError::Http(status.as_u16()));
        }
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ClientError::Shape(e.to_string()))?;
        let code = json
            .get("status")
            .and_then(Value::as_i64)
            .ok_or_else(|| ClientError::Shape("no status field".to_owned()))?;
        if code != 0 {
            return Err(ClientError::Rejected(code));
        }
        Ok(json.get("body").cloned().unwrap_or(Value::Null))
    }

    /// Re-mint the short-lived token from the persistent sessionid. Returns the rotated
    /// session: same `sessionid`, new `token`.
    pub async fn refresh(&self, s: &Session) -> Result<Session, ClientError> {
        let body = self
            .call(
                None,
                "POST",
                "session",
                "renew",
                &[
                    ("sessionid", s.sessionid.clone()),
                    ("is_secure", "1".to_owned()),
                    ("os", "android".to_owned()),
                    ("typename", "null".to_owned()),
                ],
            )
            .await?;
        let token = body
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Shape("renew returned no token".to_owned()))?
            .to_owned();
        // Renew returns only a fresh token today, but if Withings ever rotates the
        // persistent sessionid too, take the new one rather than keep a stale credential
        // that the next renew would be rejected for.
        let sessionid = body
            .get("sessionid")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| s.sessionid.clone());
        Ok(Session {
            token,
            sessionid,
            accountid: s.accountid,
            owner_userid: s.owner_userid,
        })
    }

    /// Every profile on the account.
    pub async fn profiles(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        let body = self
            .call(
                Some(s),
                "POST",
                "account",
                "getuserslist",
                &[("listmask", "0".to_owned())],
            )
            .await?;
        Ok(body
            .get("users")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// The account record (timezone, accountid).
    pub async fn account(&self, s: &Session) -> Result<Value, ClientError> {
        let body = self
            .call(
                Some(s),
                "POST",
                "account",
                "get",
                &[("applitype", "0".to_owned())],
            )
            .await?;
        Ok(body
            .get("account")
            .and_then(Value::as_array)
            .and_then(|a| a.first().cloned())
            .unwrap_or(Value::Null))
    }

    /// Devices linked to the account.
    pub async fn devices(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        let body = self
            .call(Some(s), "POST", "association", "getbyaccountid", &[])
            .await?;
        Ok(body
            .get("associations")
            .or_else(|| body.get("devices"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// One page of measurement groups for a profile, from `lastupdate` forward.
    pub async fn measures(
        &self,
        s: &Session,
        userid: i64,
        category: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<MeasPage, ClientError> {
        let body = self
            .call(
                Some(s),
                "POST",
                "measure",
                "getmeas",
                &[
                    ("userid", userid.to_string()),
                    ("category", category.to_string()),
                    ("lastupdate", lastupdate.to_string()),
                    ("offset", offset.to_string()),
                    ("limit", "200".to_owned()),
                ],
            )
            .await?;
        let groups = body
            .get("measuregrps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let updatetime = body.get("updatetime").and_then(Value::as_i64).unwrap_or(0);
        let more = truthy_more(&body);
        let next_offset = body.get("offset").and_then(Value::as_i64);
        Ok(MeasPage {
            groups,
            updatetime,
            more,
            next_offset,
            raw: body,
        })
    }

    /// One page of scores. `source` is `v2/healthindicator`, `v2/vitality` or
    /// `v2/trajectory`; the response array is named per source.
    pub async fn scores(
        &self,
        s: &Session,
        source: &str,
        userid: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<ScorePage, ClientError> {
        // Paths are not caller input in practice (the runner passes a fixed set), but the
        // allow-list check in `call` refuses anything unexpected regardless.
        let path = match source {
            "healthindicator" => "v2/healthindicator",
            "vitality" => "v2/vitality",
            "trajectory" => "v2/trajectory",
            other => return Err(ClientError::NotAllowed("POST", other.to_owned())),
        };
        let body = self
            .call(
                Some(s),
                "POST",
                path,
                "list",
                &[
                    ("userid", userid.to_string()),
                    ("lastupdate", lastupdate.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;
        let key = match source {
            "healthindicator" => "scores",
            "vitality" => "vitalities",
            _ => "trajectories",
        };
        let items = body
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let more = truthy_more(&body);
        let next_offset = body.get("offset").and_then(Value::as_i64);
        Ok(ScorePage {
            items,
            more,
            next_offset,
            raw: body,
        })
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}
