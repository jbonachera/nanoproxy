use crate::domain::{ConnectionInfo, Result};
use crate::ports::TrackingPort;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::info;
use uuid::Uuid;

/// Connection tracker implementation
pub struct ConnectionTracker {
    connections: Arc<RwLock<Vec<ConnectionInfo>>>,
}

impl ConnectionTracker {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Start background cleanup task
    pub fn start_cleanup(&self) -> tokio::task::JoinHandle<()> {
        let connections = self.connections.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(250));

            loop {
                ticker.tick().await;

                let mut conns = connections.write().await;
                let now = Instant::now();

                // Remove connections closed more than 4 seconds ago
                conns.retain(|conn| {
                    if let Some(closed_at) = conn.closed_at {
                        now.duration_since(closed_at).as_secs() < 4
                    } else {
                        true // Keep open connections
                    }
                });
            }
        })
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TrackingPort for ConnectionTracker {
    async fn track_connection(&self, info: ConnectionInfo) -> Result<()> {
        info!("{} {} (via {})", info.method, info.target, info.route);

        let mut conns = self.connections.write().await;
        conns.push(info);

        Ok(())
    }

    async fn close_connection(&self, id: Uuid) -> Result<()> {
        let mut conns = self.connections.write().await;

        if let Some(conn) = conns.iter_mut().find(|c| c.id == id) {
            conn.closed_at = Some(Instant::now());
        }

        Ok(())
    }

    async fn get_active_connections(&self) -> Result<Vec<ConnectionInfo>> {
        let conns = self.connections.read().await;
        Ok(conns.iter().filter(|c| c.closed_at.is_none()).cloned().collect())
    }
}
