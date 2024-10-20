use crate::error::*;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use url::form_urlencoded;

const GITLAB_BASE_URL: &str = "https://gitlab.com/api/v4";

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub enum PullRequestState {
    #[serde(rename = "opened")]
    Open,
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "closed")]
    Closed,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MergeRequest {
    pub title: String,
    // This is the PRs number
    #[serde(rename = "iid")]
    pub number: usize,
    pub state: PullRequestState,
    #[serde(rename = "source_branch")]
    pub source_branch: String,
    #[serde(rename = "target_branch")]
    pub target_branch: String,
    pub web_url: String,
}

impl MergeRequest {
    pub fn id(&self) -> PullRequestId {
        PullRequestId {
            url: self.web_url.clone(),
        }
    }
}

/// An id containing just enough data to uniquely identify a pull request on GitLab.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PullRequestId {
    // E.g.: https://gitlab.com/my/cool/project/-/merge_requests/123
    pub url: String,
}

impl PullRequestId {
    pub fn project(&self) -> String {
        let parts: Vec<&str> = self.url.split('/').collect();
        if parts.len() > 6
            && parts[parts.len() - 3] == "-"
            && parts[parts.len() - 2] == "merge_requests"
        {
            let project_path = parts[3..parts.len() - 3].join("/");
            return project_path;
        }
        unreachable!("Unexpected url for project: {}", self.url)
    }

    pub fn number(&self) -> usize {
        let number = self.url.rsplit('/').nth(0).unwrap();
        number
            .parse()
            .expect("Last segment should always be a number in URL.")
    }
}

pub struct GitLab {
    token: String,
    client: reqwest::Client,
}

#[derive(Deserialize, Debug)]
struct UserJson {
    username: String,
}

fn urlencode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
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
            .get(format!("{GITLAB_BASE_URL}/{endpoint}"))
            .header("PRIVATE-TOKEN", &self.token)
    }

    fn post(&self, endpoint: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{GITLAB_BASE_URL}/{endpoint}"))
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

    pub async fn get_mr(&self, project: &str, number: usize) -> Result<MergeRequest> {
        let response = self
            .get(&format!(
                "projects/{}/merge_requests/{number}",
                urlencode(project)
            ))
            .send()
            .await?;
        Ok(response.json().await?)
    }

    pub async fn create_mr(
        &self,
        project: &str,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
    ) -> Result<MergeRequest> {
        let mut form = HashMap::new();
        form.insert("source_branch", source_branch);
        form.insert("target_branch", target_branch);
        form.insert("title", title);
        form.insert("description", description);

        let response = self
            .post(&format!("projects/{}/merge_requests", urlencode(project)))
            .form(&form)
            .send()
            .await?;
        let result: MergeRequest = response.json().await?;
        Ok(result)
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
