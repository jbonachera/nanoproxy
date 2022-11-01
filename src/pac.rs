use std::error;

use js_sandbox::{JsValue, Script};
use url::Url;
const PAC_UTILS: &str = include_str!("pac_utils.js");

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

pub fn proxy_for_url(pac_file: String, url: &Url) -> Result<String> {
    let pac_payload = [pac_file.as_str(), PAC_UTILS].join("\n");

    let mut script = Script::from_string(pac_payload.as_str())?;

    let eval_result: JsValue = script.call(
        "main",
        &vec![url.to_string(), url.host().unwrap().to_string()],
    )?;

    let first_proxy = eval_result
        .to_string()
        .split(";")
        .map(|v| v.trim().split(" ").nth(1).or(Some("DIRECT")).unwrap())
        .nth(0)
        .unwrap()
        .to_string();
    Ok(format!("http://{first_proxy}"))
}
