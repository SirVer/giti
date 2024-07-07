use crate::error::*;
use chrono::{DateTime, Local};
use serde::Deserialize;
use std::env;

const GITLAB_BASE_URL: &str = "https://gitlab.com/api/v4";

#[derive(Deserialize, Debug, PartialEq)]
pub enum PullRequestState {
    #[serde(rename = "opened")]
    Open,
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "closed")]
    Closed,
}

#[derive(Deserialize, Debug)]
pub struct MergeRequest {
    pub title: String,
    // This is the PRs number
    #[serde(rename = "iid")]
    pub number: usize,
    pub state: PullRequestState,
    #[serde(rename = "source_branch")]
    pub _source_branch: String,
    #[serde(rename = "target_branch")]
    pub _target_branch: String,
    pub web_url: String,
}

pub struct GitLab {
    token: String,
    client: reqwest::Client,
}

#[derive(Deserialize, Debug)]
struct UserJson {
    username: String,
}

impl GitLab {
    pub fn new() -> Result<Self> {
        let token = env::var("GITLAB_TOKEN")?;
        Ok(Self {
            client: reqwest::Client::new(),
            token,
        })
    }

    fn get(&self, endpoint: &str) -> reqwest::RequestBuilder {
        self.client
            .get(&format!("{GITLAB_BASE_URL}/{endpoint}"))
            .header("PRIVATE-TOKEN", &self.token)
    }

    pub async fn find_user_name(&self) -> Result<String> {
        let response = self.get("user").send().await?;
        let result: UserJson = response.json().await?;
        Ok(result.username)
    }

    pub async fn search_mrs(&self, query: &str) -> Result<Vec<MergeRequest>> {
        let response = self.get(&format!("merge_requests?{query}")).send().await?;
        Ok(response.json().await?)
    }
}

// I tried the GitLab crate, but it was very limiting, so gobbling together my own little Rest
// abstraction was actually the easiest thing to do.
pub async fn find_my_mrs(
    start_date: DateTime<Local>,
    end_date: DateTime<Local>,
) -> Result<Vec<MergeRequest>> {
    let gl = GitLab::new()?;
    let start = start_date.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let end = end_date.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let user = gl.find_user_name().await?;
    let mrs = gl
        .search_mrs(&format!(
            "author_username={user}&created_after={start}&created_before={end}"
        ))
        .await?;
    Ok(mrs)
}
