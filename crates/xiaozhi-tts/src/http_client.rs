use std::time::Duration;

use reqwest::Client;
use xiaozhi_core::cloud;

pub fn build_http_client(url: &str) -> Client {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60));
    if cloud::should_bypass_proxy(url) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .unwrap_or_else(|_| Client::new())
}
