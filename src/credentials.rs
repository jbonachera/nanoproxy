use std::{collections::HashMap, num::NonZeroUsize};

use act_zero::{Actor, ActorResult, Produces};
use lru::LruCache;
use serde::{Deserialize, Serialize};
fn encode_credentials(username: &str, password: &str) -> String {
    format!("Basic {}", base64::encode(format!("{username}:{password}")))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyAuthRule {
    remote_pattern: String,
    username: String,
    password: String,
}

impl Default for ProxyAuthRule {
    fn default() -> Self {
        Self {
            remote_pattern: ".example.net".into(),
            username: "username".into(),
            password: "password".into(),
        }
    }
}

pub struct CredentialProvider {
    credentials_cache: LruCache<String, Option<String>>,
    basic_rules: HashMap<String, String>,
}

impl Actor for CredentialProvider {}
impl Default for CredentialProvider {
    fn default() -> Self {
        CredentialProvider {
            basic_rules: HashMap::new(),
            credentials_cache: LruCache::new(NonZeroUsize::new(5).unwrap()),
        }
    }
}

impl CredentialProvider {
    pub fn from_auth_rules(rules: Vec<ProxyAuthRule>) -> Self {
        let mut credential_provider = Self::default();
        rules.into_iter().for_each(|v| {
            credential_provider.basic_rules.insert(
                v.remote_pattern,
                encode_credentials(&v.username, &v.password),
            );
        });
        credential_provider
    }
    fn basic_credentials_for(&mut self, host: &str) -> Option<String> {
        if host.len() == 0 {
            return None;
        }
        match self.basic_rules.get(host) {
            Some(v) => Some(v.clone()),
            None => self.basic_credentials_for(
                host.trim_start_matches(".")
                    .trim_start_matches(|ch| ch != '.'),
            ),
        }
    }
    pub async fn credentials_for(&mut self, host: String) -> ActorResult<Option<String>> {
        let cache = self.credentials_cache.get(&host);
        match cache {
            Some(v) => Produces::ok(v.clone()),
            None => {
                let v = self.basic_credentials_for(&host);
                self.credentials_cache.put(host, v.clone());
                Produces::ok(v)
            }
        }
    }
}
