// TODO(hrapp): Upgrade chrono to get rid of this.
#![allow(deprecated)]

use crate::error::*;
use chrono::{Date, Local};
use hubcaps::search::SearchIssuesOptions;
use hubcaps::{self, Credentials};
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use tokio::stream::StreamExt;
use url;

// TODO(sirver): This state of async/await only allowed static references or owning data. So there
// is lots of cloning going on here.

#[derive(Debug)]
pub struct Branch {
    pub repo: RepoId,
    pub name: String,
}

impl Branch {
    fn from_label(repo_name: &str, label: &str) -> Self {
        let mut it = label.split(":");
        let owner = it.next().unwrap().to_string();
        let name = it.next().unwrap().to_string();
        Branch {
            repo: RepoId {
                owner: owner,
                name: repo_name.to_string(),
            },
            name,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PullRequestState {
    Open,
    Closed,
}

impl FromStr for PullRequestState {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "open" => Ok(PullRequestState::Open),
            "closed" => Ok(PullRequestState::Closed),
            _ => Err(format!("Invalid brach state: {}", s)),
        }
    }
}

#[derive(Debug)]
pub struct PullRequest {
    // Repo where this PR is opened, e.g. "SirVer/UltiSnips"
    pub target: Branch,
    pub source: Branch,
    pub number: i32,
    pub author_login: String,
    pub title: String,
    pub state: PullRequestState,
}

impl PullRequest {
    pub fn id(&self) -> PullRequestId {
        PullRequestId {
            repo: self.target.repo.clone(),
            number: self.number,
        }
    }
}

/// An id containing just enough data to uniquely identify a pull request on GitHub.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PullRequestId {
    pub repo: RepoId,
    pub number: i32,
}

impl PullRequestId {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.repo.owner, self.repo.name, self.number
        )
    }
}

impl Display for PullRequestId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            fmt,
            "{}/{}#{}",
            self.repo.owner, self.repo.name, self.number
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RepoId {
    pub owner: String,
    pub name: String,
}

type Github = hubcaps::Github;

// bug fixed version from hubcaps: http://lessis.me/hubcaps/src/hubcaps/search/mod.rs.html#229-235
pub fn repo_tuple(repository_url: &str) -> (String, String) {
    // split the last two elements off the repo url path
    let parsed = url::Url::parse(&repository_url).unwrap();
    let mut path = parsed.path().split('/').collect::<Vec<_>>();
    path.reverse();
    (path[1].to_owned(), path[0].to_owned())
}

async fn fetch_pr(
    github: Github,
    pr_id: PullRequestId,
) -> hubcaps::Result<(RepoId, hubcaps::pulls::Pull)> {
    let res = github
        .repo(pr_id.repo.owner.to_string(), pr_id.repo.name.to_string())
        .pulls()
        .get(pr_id.number as u64)
        .get()
        .await?;
    Ok((pr_id.repo, res))
}

async fn search_prs(
    github: Github,
    query: String,
) -> hubcaps::Result<Vec<(RepoId, hubcaps::pulls::Pull)>> {
    let mut search = github
        .search()
        .issues()
        .iter(query, &SearchIssuesOptions::builder().per_page(25).build());

    let mut futures = vec![];
    while let Some(Ok(result)) = search.next().await {
        let (owner, name) = repo_tuple(&result.repository_url);
        let pr_id = PullRequestId {
            repo: RepoId { owner, name },
            number: result.number as i32,
        };
        futures.push(fetch_pr(github.clone(), pr_id));
    }

    let mut results = vec![];
    for rv in futures::future::join_all(futures).await {
        results.push(rv?);
    }
    Ok(results)
}

async fn find_login_name(github: Github) -> hubcaps::Result<String> {
    Ok(github.users().authenticated().await?.login)
}

async fn run_find_assigned_prs(
    github: Github,
) -> hubcaps::Result<Vec<(RepoId, hubcaps::pulls::Pull)>> {
    let login = find_login_name(github.clone()).await?;
    let query = format!("is:pr is:open archived:false assignee:{}", login);
    let res = search_prs(github.clone(), query).await?;
    Ok(res)
}

fn search_result_to_pull_requests(prs: Vec<(RepoId, hubcaps::pulls::Pull)>) -> Vec<PullRequest> {
    prs.iter()
        .map(|(pr_repo, pr)| PullRequest {
            source: Branch::from_label(&pr_repo.name, &pr.head.label),
            target: Branch::from_label(&pr_repo.name, &pr.base.label),
            number: pr.number as i32,
            author_login: pr.user.login.clone(),
            title: pr.title.clone(),
            state: PullRequestState::from_str(&pr.state).unwrap(),
        })
        .collect()
}

pub async fn find_assigned_prs(repo: Option<&RepoId>) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;
    let repo = repo.map(|r| r.clone());

    async move {
        let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)))
            .expect("GitHub could not be constructed");
        let mut prs = run_find_assigned_prs(github.clone())
            .await
            .expect("run_find_assigned_prs() did not succeed.");
        prs.sort_by_key(|(_, pr)| pr.number);

        let new_result = search_result_to_pull_requests(
            prs.into_iter()
                .filter(|(pr_repo, _)| match repo {
                    None => true,
                    Some(ref r) => pr_repo == r,
                })
                .collect(),
        );

        Ok(new_result)
    }
    .await
}

pub async fn find_my_prs(
    start_date: Date<Local>,
    end_date: Date<Local>,
) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    async move {
        let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)))
            .expect("GitHub could not be constructed");

        let login = find_login_name(github.clone())
            .await
            .expect("Could not find GitHub login.");
        let query = format!(
            "is:pr author:{} created:{}..{}",
            login,
            start_date.format("%Y-%m-%d"),
            end_date.format("%Y-%m-%d")
        );
        let prs = search_prs(github.clone(), query)
            .await
            .expect("Could not search for PRs.");

        let mut results = search_result_to_pull_requests(prs);
        results.sort_by_key(|pr| (pr.target.repo.name.clone(), pr.number));
        Ok(results)
    }
    .await
}

pub async fn create_pr(
    repo: &RepoId,
    pull_options: hubcaps::pulls::PullOptions,
) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let repo_clone = repo.clone();
    let pr = async move {
        let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)))
            .expect("GitHub could not be constructed");
        let result = github
            .repo(repo_clone.owner.to_string(), repo_clone.name.to_string())
            .pulls()
            .create(&pull_options)
            .await;
        result
    }
    .await?;

    Ok(PullRequest {
        source: Branch::from_label(&repo.name, &pr.head.label),
        target: Branch::from_label(&repo.name, &pr.base.label),
        number: pr.number as i32,
        author_login: pr.user.login.clone(),
        title: pr.title.clone(),
        state: PullRequestState::from_str(&pr.state).unwrap(),
    })
}

pub async fn get_pr(pr_id: &PullRequestId) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let pr_id_clone = pr_id.clone();
    let pr = async move {
        let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)))
            .expect("GitHub could not be constructed");
        let (_, pr) = fetch_pr(github, pr_id_clone)
            .await
            .expect("fetch_pr did not complete.");
        pr
    }
    .await;

    Ok(PullRequest {
        source: Branch::from_label(&pr_id.repo.name, &pr.head.label),
        target: Branch::from_label(&pr_id.repo.name, &pr.base.label),
        number: pr.number as i32,
        author_login: pr.user.login.clone(),
        title: pr.title.clone(),
        state: PullRequestState::from_str(&pr.state).unwrap(),
    })
}

pub fn get_pull_request_template(workdir: &Path) -> Option<String> {
    for sub_path in &[".github", "docs", "."] {
        let files = match ::std::fs::read_dir(&workdir.join(sub_path)) {
            Err(_) => continue,
            Ok(r) => r,
        };
        for f in files {
            let p = match f {
                Err(_) => continue,
                Ok(d) => d.path(),
            };
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(String::new)
                .to_lowercase();
            if stem == "pull_request_template" {
                return ::std::fs::read_to_string(p)
                    .map(|s| Some(s))
                    .unwrap_or(None);
            }
        }
    }
    None
}
