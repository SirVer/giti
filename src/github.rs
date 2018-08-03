use hubcaps::search::SearchIssuesOptions;
use hubcaps::{self, Credentials};
use tokio_core::reactor::Core;
use std::env;
use futures::{self, prelude::*};
use hyper;
use error::*;
use hyper_tls;
use url;

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

#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
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

#[async]
fn fetch_pr(github: Github, repo: Repo, number: u64) -> hubcaps::Result<(Repo, hubcaps::pulls::Pull)> {
    let res = await!(github.repo(repo.owner.to_string(), repo.name.to_string()).pulls().get(number).get())?;
    Ok((repo, res))
}

#[async]
fn find_assigned_pr_info(
    github: Github,
    login: String
) -> hubcaps::Result<Vec<(Repo, hubcaps::pulls::Pull)>> {
    let search = github.search().issues().iter(
        format!(
            "is:pr is:open archived:false assignee:{}",
            login,
        ),
        &SearchIssuesOptions::builder().per_page(25).build(),
    );

    let mut requests = vec![];
    #[async]
    for result in search {
        let (owner, name) = repo_tuple(&result.repository_url);
        let repo = Repo { owner, name };
        let mut future = fetch_pr(github.clone(), repo, result.number);
        requests.push(future);
    }
    Ok(await!(futures::future::join_all(requests))?)
}

#[async]
fn find_login_name(github: Github) -> hubcaps::Result<String> {
    Ok(await!(github.users().authenticated())?.login)
}

#[async]
fn run(github: Github) -> hubcaps::Result<Vec<(Repo, hubcaps::pulls::Pull)>> {
    let login = await!(find_login_name(github.clone()))?;
    let res = await!(find_assigned_pr_info(github.clone(), login))?;
    Ok(res)
}

pub fn find_assigned_prs(repo: Option<&Repo>) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    let mut core = Core::new().expect("reactor fail");
    let github = Github::new(
        "SirVer_giti/unspecified",
        Some(Credentials::Token(token)),
        &core.handle(),
    );

    let mut prs = core.run(run(github.clone()))?;
    prs.sort_by_key(|(_, pr)| pr.number);

    let result = prs.iter()
        .filter(|(pr_repo, _)| match repo {
            None => true,
            Some(ref r) => pr_repo == *r,
        })
        .map(|(pr_repo, pr)| PullRequest {
            source: Branch::from_label(&pr_repo.name, &pr.head.label),
            target: Branch::from_label(&pr_repo.name, &pr.base.label),
            number: pr.number as i32,
            author_login: pr.user.login.clone(),
            title: pr.title.clone(),
        })
        .collect::<Vec<_>>();

    Ok(result)
}

pub fn get_pr(repo: &Repo, pr: i32) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let mut core = Core::new().expect("reactor fail");
    let github = Github::new(
        "SirVer_giti/unspecified",
        Some(Credentials::Token(token)),
        &core.handle(),
    );

    let (_, pr) = core.run(fetch_pr(github, repo.clone(), pr as u64))?;
    Ok(PullRequest {
        source: Branch::from_label(&repo.name, &pr.head.label),
        target: Branch::from_label(&repo.name, &pr.base.label),
        number: pr.number as i32,
        author_login: pr.user.login.clone(),
        title: pr.title.clone(),
    })
}
