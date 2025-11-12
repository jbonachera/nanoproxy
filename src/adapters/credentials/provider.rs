use crate::domain::{AuthRule, Credentials, ProxyError, Result};
use crate::ports::CredentialsPort;
use async_trait::async_trait;
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Credential provider implementation
pub struct CredentialProvider {
    rules: Arc<RwLock<HashMap<String, (String, String)>>>,
    cache: Arc<RwLock<LruCache<String, Option<Credentials>>>>,
}

impl CredentialProvider {
    pub fn new(auth_rules: Vec<AuthRule>) -> Self {
        let mut rules_map = HashMap::new();

        for rule in auth_rules {
            // Execute password command once at initialization
            let password = Self::execute_password_command(&rule.password_command).unwrap_or_else(|_| String::new());
            if password.len() == 0 {
                log::error!("Password command returned an empty password.");
                panic!("invalid password command")
            }
            rules_map.insert(rule.remote_pattern, (rule.username, password));
        }

        Self {
            rules: Arc::new(RwLock::new(rules_map)),
            cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(5).unwrap()))),
        }
    }

    fn execute_password_command(cmd: &str) -> Result<String> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| ProxyError::AuthenticationFailed(format!("Command failed: {}", e)))?;

        String::from_utf8(output.stdout)
            .map(|s| s.trim_end().to_string())
            .map_err(|e| ProxyError::AuthenticationFailed(format!("Invalid UTF-8: {}", e)))
    }

    async fn find_credentials_for_host(&self, host: &str) -> Option<Credentials> {
        if host.is_empty() {
            return None;
        }

        let rules = self.rules.read().await;

        // Try exact match first
        if let Some((username, password)) = rules.get(host) {
            log::debug!("Found credentials for host {}: {}", host, username);
            return Some(Credentials::new(username.clone(), password.clone()));
        }

        // Try pattern matching (remove leading segments)
        let trimmed = host.trim_start_matches('.').trim_start_matches(|ch| ch != '.');
        if trimmed != host && !trimmed.is_empty() {
            drop(rules);
            return Box::pin(self.find_credentials_for_host(trimmed)).await;
        }

        None
    }
}

#[async_trait]
impl CredentialsPort for CredentialProvider {
    async fn get_credentials(&self, host: &str) -> Result<Option<Credentials>> {
        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(host) {
                return Ok(cached.clone());
            }
        }

        let creds = self.find_credentials_for_host(host).await;

        {
            let mut cache = self.cache.write().await;
            cache.put(host.to_string(), creds.clone());
        }
        Ok(creds)
    }

    async fn clear_cache(&self) -> Result<()> {
        let mut cache = self.cache.write().await;
        cache.clear();
        Ok(())
    }
}
