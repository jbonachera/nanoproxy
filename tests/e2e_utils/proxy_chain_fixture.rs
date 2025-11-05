use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::e2e_utils::intermediate_proxy::IntermediateProxy;

pub fn create_pac_script(proxy_host: &str, proxy_port: u16) -> String {
    format!(
        r#"
function FindProxyForURL(url, host) {{
    return "PROXY {proxy_host}:{proxy_port}";
}}
"#,
        proxy_host = proxy_host,
        proxy_port = proxy_port
    )
}

pub struct ProxyChainFixture {
    _intermediate_handle: JoinHandle<()>,
    pac_file: std::path::PathBuf,
    _pac_file_cleanup: Arc<std::path::PathBuf>,
}

impl ProxyChainFixture {
    pub async fn setup(intermediate_port: u16) -> Result<Self, Box<dyn std::error::Error>> {
        let intermediate = IntermediateProxy::new(intermediate_port).await?;
        let intermediate_proxy_addr = intermediate.local_addr()?;
        let intermediate_handle = intermediate.run().await;

        sleep(Duration::from_millis(50)).await;

        let pac_script = create_pac_script("127.0.0.1", intermediate_proxy_addr.port());
        let pac_file = std::env::temp_dir().join(format!("nanoproxy_test_{}.pac", std::process::id()));
        std::fs::write(&pac_file, &pac_script)?;

        Ok(Self {
            _intermediate_handle: intermediate_handle,
            pac_file: pac_file.clone(),
            _pac_file_cleanup: Arc::new(pac_file),
        })
    }

    pub fn pac_file_path(&self) -> &std::path::Path {
        &self.pac_file
    }
}

impl Drop for ProxyChainFixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.pac_file).ok();
    }
}
