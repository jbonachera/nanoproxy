use crate::domain::Result;
use async_trait::async_trait;

/// Port for establishing bidirectional tunnels (for CONNECT method)
#[async_trait]
#[allow(dead_code)] // Prepared for testing and alternative implementations
pub trait TunnelPort: Send + Sync {
    /// Establish a bidirectional tunnel between client and server
    ///
    /// This is used for CONNECT requests where we need to relay
    /// raw TCP traffic between the client and the destination.
    async fn establish_tunnel(&self, client: Box<dyn TunnelStream>, server: Box<dyn TunnelStream>) -> Result<()>;
}

/// Trait for a stream that can be used in a tunnel
#[async_trait]
#[allow(dead_code)] // Prepared for testing and alternative implementations
pub trait TunnelStream: Send + Sync {
    /// Copy data bidirectionally with another stream
    async fn copy_bidirectional(&mut self, other: &mut dyn TunnelStream) -> Result<(u64, u64)>;
}
