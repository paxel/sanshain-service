//! The outbound POST of a webhook delivery (#47): one reqwest client, built
//! once with the delivery timeout and the process trust store, behind the
//! domain's [`WebhookSender`] port.

use crate::domain::ports::{WebhookRequest, WebhookResponse, WebhookSender};
use std::time::Duration;

#[derive(Clone)]
pub struct ReqwestWebhookSender {
    client: reqwest::Client,
}

impl ReqwestWebhookSender {
    /// A client that gives a receiver `timeout` for the whole exchange and
    /// never follows a redirect: a hook URL points at the receiver itself,
    /// and a redirect could otherwise walk the signed POST to a host the
    /// allow-list never admitted.
    pub fn new(timeout: Duration) -> Result<Self, String> {
        let client = reqwest::ClientBuilder::new()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("Sanshain/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

impl WebhookSender for ReqwestWebhookSender {
    async fn send(&self, request: WebhookRequest) -> Result<WebhookResponse, String> {
        let mut builder = self
            .client
            .post(&request.url)
            .header("Content-Type", "application/json");
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .body(request.body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(WebhookResponse {
            status: response.status().as_u16(),
        })
    }
}
