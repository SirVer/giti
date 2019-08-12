// Extremely helpful was this: https://jsdw.me/posts/rust-asyncawait-preview/

use crate::error::*;
use chrono::{Date, Local};
use hubcaps::search::SearchIssuesOptions;
use hubcaps::{self, Credentials};
use hyper;
use hyper_tls;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use tokio::await;
use tokio::prelude::*;
use tokio_async_await::compat::backward::Compat;
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

type Github = hubcaps::Github<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>;

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
    let res = await!(github
        .repo(pr_id.repo.owner.to_string(), pr_id.repo.name.to_string())
        .pulls()
        .get(pr_id.number as u64)
        .get())?;
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
    while let Some(Ok(result)) = await!(search.next()) {
        let (owner, name) = repo_tuple(&result.repository_url);
        let pr_id = PullRequestId {
            repo: RepoId { owner, name },
            number: result.number as i32,
        };
        futures.push(Compat::new(fetch_pr(github.clone(), pr_id)));
    }

    let mut results = vec![];
    for rv in await!(tokio::prelude::future::join_all(futures))? {
        results.push(rv);
    }
    Ok(results)
}

async fn find_login_name(github: Github) -> hubcaps::Result<String> {
    Ok(await!(github.users().authenticated())?.login)
}

async fn run_find_assigned_prs(
    github: Github,
) -> hubcaps::Result<Vec<(RepoId, hubcaps::pulls::Pull)>> {
    let login = await!(find_login_name(github.clone()))?;
    let query = format!("is:pr is:open archived:false assignee:{}", login);
    let res = await!(search_prs(github.clone(), query))?;
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

pub fn find_assigned_prs(repo: Option<&RepoId>) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    let repo = repo.map(|r| r.clone());
    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));
            let mut prs = await!(run_find_assigned_prs(github.clone()))
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

            tx.lock().unwrap().send(new_result).unwrap();
        },
    );

    Ok(rx.recv().unwrap())
}

pub fn find_my_prs(
    start_date: Date<Local>,
    end_date: Date<Local>,
) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));

            let login =
                await!(find_login_name(github.clone())).expect("Could not find GitHub login.");
            let query = format!(
                "is:pr author:{} created:{}..{}",
                login,
                start_date.format("%Y-%m-%d"),
                end_date.format("%Y-%m-%d")
            );
            let prs = await!(search_prs(github.clone(), query)).expect("Could not search for PRs.");

            let mut results = search_result_to_pull_requests(prs);
            results.sort_by_key(|pr| (pr.target.repo.name.clone(), pr.number));

            tx.lock().unwrap().send(results).unwrap();
        },
    );

    Ok(rx.recv().unwrap())
}

pub fn create_pr(repo: &RepoId, pull_options: hubcaps::pulls::PullOptions) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let repo_clone = repo.clone();
    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));
            let result = await!(github
                .repo(repo_clone.owner.to_string(), repo_clone.name.to_string())
                .pulls()
                .create(&pull_options));
            tx.lock().unwrap().send(result).unwrap();
        },
    );

    let pr = rx.recv().unwrap()?;
    Ok(PullRequest {
        source: Branch::from_label(&repo.name, &pr.head.label),
        target: Branch::from_label(&repo.name, &pr.base.label),
        number: pr.number as i32,
        author_login: pr.user.login.clone(),
        title: pr.title.clone(),
        state: PullRequestState::from_str(&pr.state).unwrap(),
    })
}

pub fn get_pr(pr_id: &PullRequestId) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    let pr_id_clone = pr_id.clone();
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));
            let (_, pr) =
                await!(fetch_pr(github, pr_id_clone)).expect("fetch_pr did not complete.");
            tx.lock().unwrap().send(pr).unwrap();
        },
    );

    let pr = rx.recv().unwrap();
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
