use hubcaps::search::SearchIssuesOptions;
use hubcaps::{self, Credentials};
use tokio_core::reactor::Core;
use std::env;
use futures::prelude::*;
use hyper;
use error::*;
use hyper_tls;

// TODO(sirver): This state of async/await only allowed static references or owning data. So there
// is lots of cloning going on here.

#[derive(Debug)]
pub struct Branch {
    pub repo: Repo,
    pub name: String,
}

impl Branch {
    fn from_label(repo_name: &str, label: &str) -> Self {
        let mut it = label.split(":");
        let owner = it.next().unwrap().to_string();
        let name = it.next().unwrap().to_string();
        Branch {
            repo: Repo {
                owner: owner,
                name: repo_name.to_string(),
            },
            name,
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
}

#[derive(Debug, Clone)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

type Github = hubcaps::Github<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>;

#[async]
fn fetch_pr(github: Github, repo: Repo, number: u64) -> hubcaps::Result<hubcaps::pulls::Pull> {
    let res = await!(github.repo(repo.owner, repo.name).pulls().get(number).get())?;
    Ok(res)
}

#[async]
fn find_assigned_pr_info(
    github: Github,
    login: String,
    repo: Repo,
) -> hubcaps::Result<Vec<hubcaps::pulls::Pull>> {
    let search = github.search().issues().iter(
        format!(
            "is:pr is:open archived:false assignee:{} repo:{}/{}",
            login, repo.owner, repo.name
        ),
        &SearchIssuesOptions::builder().per_page(25).build(),
    );

    // TODO(sirver): I am actually not sure if this is actually faster than just await!() in the
    // loop?
    let mut requests = vec![];
    #[async]
    for result in search {
        requests.push(fetch_pr(github.clone(), repo.clone(), result.number));
    }

    let mut items = vec![];
    for r in requests {
        let res = await!(r)?;
        items.push(res);
    }
    Ok(items)
}

#[async]
fn find_login_name(github: Github) -> hubcaps::Result<String> {
    Ok(await!(github.users().authenticated())?.login)
}

#[async]
fn run(github: Github, repo: Repo) -> hubcaps::Result<Vec<hubcaps::pulls::Pull>> {
    let login = await!(find_login_name(github.clone()))?;
    let res = await!(find_assigned_pr_info(github.clone(), login, repo))?;
    Ok(res)
}

pub fn find_assigned_prs(repo: &Repo) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    let mut core = Core::new().expect("reactor fail");
    let github = Github::new(
        "SirVer_giti/unspecified",
        Some(Credentials::Token(token)),
        &core.handle(),
    );

    let mut prs = core.run(run(github.clone(), repo.clone()))?;
    prs.sort_by_key(|pr| pr.number);

    let result = prs.iter()
        .map(|pr| PullRequest {
            source: Branch::from_label(&repo.name, &pr.head.label),
            target: Branch::from_label(&repo.name, &pr.base.label),
            number: pr.number as i32,
            author_login: pr.user.login.clone(),
            title: pr.title.clone(),
        })
        .collect::<Vec<_>>();

    Ok(result)
}
