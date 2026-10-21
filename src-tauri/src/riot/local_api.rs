//! Local Riot Client HTTPS API (127.0.0.1, self-signed cert, basic auth).
//! Provides entitlements/access tokens, presences, and region-locale. See spec §1-2, §4.

use crate::riot::error::{Error, Result};
use crate::riot::lockfile::Lockfile;
use crate::riot::presence::RawPresence;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

/// Entitlements + access token from the local client.
#[derive(Debug, Clone, Deserialize)]
pub struct EntitlementsToken {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    /// The entitlements JWT (`X-Riot-Entitlements-JWT`).
    pub token: String,
    /// The local player's own PUUID.
    pub subject: String,
}

/// `/riotclient/region-locale` response.
#[derive(Debug, Clone, Deserialize)]
pub struct RegionLocale {
    pub region: String,
}

/// A client scoped to the local Riot Client. Accepts the self-signed cert — this client
/// is only ever pointed at 127.0.0.1.
pub struct LocalClient {
    http: reqwest::Client,
    lockfile: Lockfile,
}

impl LocalClient {
    pub fn new(lockfile: Lockfile) -> Result<Self> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { http, lockfile })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.lockfile.local_base_url(), path)
    }

    /// GET a local endpoint with basic auth, mapping "not ready" bodies to `NotReady`.
    async fn get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(self.url(path))
            .header("Authorization", self.lockfile.basic_auth_header())
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if is_not_ready(&body) || status.as_u16() == 404 {
            vlt_log!("presence", "local {path} not ready (status {})", status.as_u16());
            return Err(Error::NotReady);
        }
        if !status.is_success() {
            return Err(Error::Http(format!("local {path} -> {status}")));
        }
        Ok(body)
    }

    /// GET with retry (3 attempts, 5s apart) for the client-still-starting case.
    async fn get_retry(&self, path: &str) -> Result<Value> {
        let mut last = Error::NotReady;
        for attempt in 0..3 {
            match self.get(path).await {
                Ok(v) => return Ok(v),
                Err(e) if is_starting_up(&e) => {
                    last = e;
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Fetch entitlements + access token (retries while the client is starting).
    pub async fn entitlements(&self) -> Result<EntitlementsToken> {
        let body = self.get_retry("/entitlements/v1/token").await?;
        serde_json::from_value(body).map_err(Error::from)
    }

    /// Fetch the client's region (for shard/host construction).
    pub async fn region_locale(&self) -> Result<RegionLocale> {
        let body = self.get_retry("/riotclient/region-locale").await?;
        serde_json::from_value(body).map_err(Error::from)
    }

    /// Try external-sessions for the authoritative Valorant deployment shard.
    pub async fn deployment_region(&self) -> Option<String> {
        let body = self.get("/product-session/v1/external-sessions").await.ok()?;
        extract_region_from_sessions(&body)
    }

    /// Authoritative region discovery: external-sessions first (-ares-deployment), then region-locale fallback.
    pub async fn discover_region(&self) -> Result<String> {
        if let Some(region) = self.deployment_region().await {
            vlt_log!("conn", "authoritative region from external-sessions: {region}");
            return Ok(region);
        }
        let region = self.region_locale().await?.region;
        vlt_log!("conn", "fallback region from region-locale: {region}");
        Ok(region)
    }

    /// Fetch all current presences, optionally handing back the untouched response body
    /// alongside the parsed roster. The body is cloned only when `raw` is set, so the
    /// normal path (`raw == false`) pays nothing — it exists for the dev-only capture
    /// (`docs/testing.md`), where unknown/undecoded fields are what needs inspecting.
    pub async fn presences_with_raw(&self, raw: bool) -> Result<(Vec<RawPresence>, Option<Value>)> {
        let body = self.get("/chat/v4/presences").await?;
        let captured = if raw { Some(body.clone()) } else { None };
        // Deserialized by reference: the roster is read up to 60x/min in agent select, so the
        // array is never cloned out of the body first.
        let presences = body
            .get("presences")
            .and_then(|p| p.as_array())
            .map(|list| list.iter().filter_map(|v| RawPresence::deserialize(v).ok()).collect())
            .unwrap_or_default();
        Ok((presences, captured))
    }
}

/// Whether a failed local GET means the client is still starting, so the bounded retry should
/// take another attempt. Spec §1 counts ANY non-200 (not just 404) or an RPC_ERROR body as "not
/// ready yet" while the client boots, which is what `Error::Http` carries here. Everything else
/// (a malformed body, a transport failure) is a different problem and fails at once. Pure.
fn is_starting_up(err: &Error) -> bool {
    matches!(err, Error::NotReady | Error::Http(_))
}

/// True if a local response body is a "client still starting" placeholder.
fn is_not_ready(body: &Value) -> bool {
    if let Some(code) = body.get("errorCode").and_then(|v| v.as_str()) {
        if code == "RPC_ERROR" {
            return true;
        }
    }
    if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
        return msg.contains("not ready") || msg.contains("Invalid URI");
    }
    false
}

/// Extract the active Valorant deployment shard from an external-sessions payload.
pub fn extract_region_from_sessions(value: &Value) -> Option<String> {
    if let Value::Object(map) = value {
        for (_k, session) in map {
            let is_val = session
                .get("productId")
                .or_else(|| session.get("product_id"))
                .and_then(|v| v.as_str())
                .map(|p| p.eq_ignore_ascii_case("valorant"))
                .unwrap_or(false);

            if is_val {
                if let Some(args) = session.pointer("/launchConfiguration/arguments").and_then(|v| v.as_array()) {
                    for arg in args {
                        if let Some(s) = arg.as_str() {
                            if let Some(shard) = extract_ares_deployment(s) {
                                return Some(shard);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback scan across text
    extract_ares_deployment(&value.to_string())
}

fn extract_ares_deployment(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let flag = "-ares-deployment=";
    if let Some(idx) = lower.find(flag) {
        let rest = &lower[idx + flag.len()..];
        let shard: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        if !shard.is_empty() {
            return Some(shard);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_ready_bodies_are_detected() {
        for (body, not_ready) in [
            (json!({ "errorCode": "RPC_ERROR" }), true),
            (json!({ "message": "Entitlements token is not ready yet" }), true),
            (json!({ "message": "Invalid URI format" }), true),
            (json!({ "accessToken": "x", "token": "y", "subject": "z" }), false),
        ] {
            assert_eq!(is_not_ready(&body), not_ready, "{body}");
        }
    }

    #[test]
    fn any_non_200_is_retried_while_the_client_starts() {
        // Spec §1: during startup every non-200 (not only 404) reads as "not ready yet".
        assert!(is_starting_up(&Error::NotReady));
        assert!(is_starting_up(&Error::Http("local /x -> 503".into())));
    }

    #[test]
    fn parses_entitlements_shape() {
        let e: EntitlementsToken =
            serde_json::from_value(json!({ "accessToken": "a", "token": "t", "subject": "s", "issuer": "i", "entitlements": [] })).unwrap();
        assert_eq!(e.access_token, "a");
        assert_eq!(e.token, "t");
        assert_eq!(e.subject, "s");
    }

    #[test]
    fn extracts_deployment_from_external_sessions() {
        let payload = json!({
            "session-guid-123": {
                "productId": "valorant",
                "launchConfiguration": {
                    "arguments": [
                        "-from-launcher",
                        "-ares-deployment=ap",
                        "-config-endpoint=https://shared.ap.a.pvp.net"
                    ]
                }
            }
        });
        assert_eq!(extract_region_from_sessions(&payload), Some("ap".to_string()));

        let empty = json!({});
        assert_eq!(extract_region_from_sessions(&empty), None);
    }
}
