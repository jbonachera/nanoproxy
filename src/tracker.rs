use std::time::Duration;

use act_zero::timer::Tick;
use async_trait::async_trait;
use tracing::info;

use tokio::time::Instant;

use uuid::Uuid;

use act_zero::runtimes::tokio::Timer;
use act_zero::*;

pub struct StreamInfo {
    pub id: Uuid,
    pub method: String,
    pub remote: String,
    pub upstream: String,
    pub opened_at: Instant,
    pub closed_at: Option<Instant>,
}
pub struct ConnectionTracker {
    items: Vec<StreamInfo>,
    timer: Timer,
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self {
            items: Default::default(),
            timer: Timer::default(),
        }
    }
}

#[async_trait]
impl Actor for ConnectionTracker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.timer
            .set_interval_weak(addr.downgrade().clone(), Duration::from_millis(250));
        Produces::ok(())
    }
}

#[async_trait]
impl Tick for ConnectionTracker {
    async fn tick(&mut self) -> ActorResult<()> {
        if self.timer.tick() {
            self.items
                .retain(|v| v.closed_at.is_none() || v.closed_at.unwrap().elapsed().as_secs() < 4);
        }
        Produces::ok(())
    }
}

impl ConnectionTracker {
    pub async fn push(&mut self, info: StreamInfo) -> ActorResult<()> {
        info!("{} {} (via {})", info.method, info.remote, info.upstream);
        self.items.push(info);

        Produces::ok(())
    }
    pub async fn remove(&mut self, id: Uuid) -> ActorResult<()> {
        match self.items.iter().position(|v| v.id == id) {
            Some(pos) => {
                self.items[pos].closed_at = Some(Instant::now());
            }
            None => {}
        }
        Produces::ok(())
    }
}
