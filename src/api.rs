use serde_json::Value;
use std::error::Error;
use thiserror::Error;

use serde::{Deserialize, Serialize};

pub struct Api {
    pub country: &'static str,
    pub client: reqwest::Client,
}

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("request error {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("json error {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("no matches found")]
    NoMatchesFound,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse {
    pub result_count: usize,
    pub results: Vec<AppInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub artist_id: u64,
    pub artist_name: String,
}

impl Api {
    pub fn new(country: &'static str) -> Api {
        Api {
            country,
            client: reqwest::Client::new(),
        }
    }

    pub async fn lookup(&self, identifier: &str) -> Result<AppInfo, ApiError> {
        let url = format!(
            "https://itunes.apple.com/lookup?bundleId={}&country={}",
            identifier, self.country
        );
        let response = self.client.get(&url).send().await?;
        let json = response.json::<ApiResponse>().await?;
        let info = json
            .results
            .get(0)
            .cloned()
            .ok_or(ApiError::NoMatchesFound)?;
        Ok(info)
    }
}
