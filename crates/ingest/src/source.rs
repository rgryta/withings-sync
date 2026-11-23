//! What the runner needs from Withings, as a trait so a test can stand in a fake. The
//! real client implements it by delegating; nothing here adds behaviour.

use serde_json::Value;
use withings_client::{Client, ClientError, MeasPage, ScorePage, Session};

#[allow(async_fn_in_trait)]
pub trait Source {
    async fn login(&self, s: &Session) -> Result<Session, ClientError>;
    async fn profiles(&self, s: &Session) -> Result<Vec<Value>, ClientError>;
    async fn account(&self, s: &Session) -> Result<Value, ClientError>;
    async fn devices(&self, s: &Session) -> Result<Vec<Value>, ClientError>;
    async fn measures(
        &self,
        s: &Session,
        userid: i64,
        category: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<MeasPage, ClientError>;
    async fn scores(
        &self,
        s: &Session,
        source: &str,
        userid: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<ScorePage, ClientError>;
}

impl Source for Client {
    async fn login(&self, s: &Session) -> Result<Session, ClientError> {
        Client::login(self, s).await
    }
    async fn profiles(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        Client::profiles(self, s).await
    }
    async fn account(&self, s: &Session) -> Result<Value, ClientError> {
        Client::account(self, s).await
    }
    async fn devices(&self, s: &Session) -> Result<Vec<Value>, ClientError> {
        Client::devices(self, s).await
    }
    async fn measures(
        &self,
        s: &Session,
        userid: i64,
        category: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<MeasPage, ClientError> {
        Client::measures(self, s, userid, category, lastupdate, offset).await
    }
    async fn scores(
        &self,
        s: &Session,
        source: &str,
        userid: i64,
        lastupdate: i64,
        offset: i64,
    ) -> Result<ScorePage, ClientError> {
        Client::scores(self, s, source, userid, lastupdate, offset).await
    }
}
