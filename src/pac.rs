use std::error;

use js_sandbox::{JsValue, Script};
use url::Url;
const PAC_UTILS: &str = include_str!("pac_utils.js");

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

pub fn proxy_for_url(pac_file: String, url: &Url) -> Result<Vec<String>> {
    let pac_payload = [pac_file.as_str(), PAC_UTILS].join("\n");

    let mut script = Script::from_string(pac_payload.as_str())?;
    let host = url.host().unwrap().to_string();

    let eval_result: JsValue = script.call(
        "FindProxyForURL",
        (url.to_string(), host),
    )?;

    let proxies: Vec<String> = eval_result
        .to_string().replace('"', "")
        .split(";")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| {
            let parts: Vec<&str> = v.split_whitespace().collect();
            match parts.get(0) {
                Some(&"DIRECT") => "direct://".to_owned(),
                Some(&"PROXY") => {
                    if let Some(proxy) = parts.get(1) {
                        format!("http://{}", proxy)
                    } else {
                        "direct://".to_owned()
                    }
                },
                _ => "direct://".to_owned(),
            }
        })
        .collect();

    if proxies.is_empty() {
        Ok(vec!["direct://".to_owned()])
    } else {
        Ok(proxies)
    }
}
