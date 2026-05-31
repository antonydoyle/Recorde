pub mod github;
pub mod gitlab;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState { Open, Draft, Merged, Closed }

impl PrState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open   => "Open",
            Self::Draft  => "Draft",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
    pub fn from_str_github(s: &str, draft: bool) -> Self {
        if draft { return Self::Draft; }
        match s { "closed" => Self::Closed, "merged" => Self::Merged, _ => Self::Open }
    }
    pub fn from_str_gitlab(s: &str, draft: bool) -> Self {
        if draft { return Self::Draft; }
        match s { "closed" | "locked" => Self::Closed, "merged" => Self::Merged, _ => Self::Open }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState { Success, Failure, Pending }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewState { Approved, ChangesRequested, Pending }

/// A pull request / merge request from any provider.
#[derive(Debug, Clone)]
pub struct PrItem {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub state: PrState,
    pub head_ref: String,
    pub base_ref: String,
    pub url: String,
    pub updated_ts: i64,
    pub label_names: Vec<String>,
    pub check_state: Option<CheckState>,
    pub review_state: Option<ReviewState>,
    pub body: String,
    pub comments: u32,
    pub changed_files: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrFilter { #[default] Open, Closed, All }

impl PrFilter {
    pub fn label(self) -> &'static str {
        match self { Self::Open => "Open", Self::Closed => "Closed", Self::All => "All" }
    }
    pub fn matches(&self, s: &PrState) -> bool {
        match self {
            Self::All    => true,
            Self::Open   => matches!(s, PrState::Open | PrState::Draft),
            Self::Closed => matches!(s, PrState::Closed | PrState::Merged),
        }
    }
}

/// Parse an ISO-8601 timestamp ("2024-01-15T10:30:00Z") into a Unix timestamp.
pub fn parse_iso8601(s: &str) -> i64 {
    if s.len() < 19 { return 0; }
    let b = s.as_bytes();
    let d = |slice: &[u8]| slice.iter().fold(0i64, |a, &c| a * 10 + (c - b'0') as i64);
    let (year, month, day) = (d(&b[0..4]), d(&b[5..7]), d(&b[8..10]));
    let (hour, min, sec)   = (d(&b[11..13]), d(&b[14..16]), d(&b[17..19]));
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dim  = [0i64,31,28,31,30,31,30,31,31,30,31,30,31];
    let mut days = (year-1970)*365 + (year-1969)/4 - (year-1901)/100 + (year-1601)/400;
    for m in 1..month { days += dim[m as usize] + if m==2 && leap(year) {1} else {0}; }
    days += day - 1;
    days * 86400 + hour * 3600 + min * 60 + sec
}

// ── Phase 13 types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState { Open, Closed }

#[derive(Debug, Clone)]
pub struct IssueItem {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub state: IssueState,
    pub url: String,
    pub label_names: Vec<String>,
    pub body: String,
    pub comments: u32,
    pub updated_ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IssueFilter { #[default] Open, Closed, All }

impl IssueFilter {
    pub fn label(self) -> &'static str {
        match self { Self::Open => "Open", Self::Closed => "Closed", Self::All => "All" }
    }
    pub fn api_state(self) -> &'static str {
        match self { Self::Open => "open", Self::Closed => "closed", Self::All => "all" }
    }
    pub fn matches(&self, s: &IssueState) -> bool {
        match self {
            Self::All    => true,
            Self::Open   => *s == IssueState::Open,
            Self::Closed => *s == IssueState::Closed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus { Success, Failure, Running, Pending, Cancelled }

impl CiStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Success   => "passed",
            Self::Failure   => "failed",
            Self::Running   => "running",
            Self::Pending   => "pending",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn from_github(conclusion: &str, status: &str) -> Self {
        match conclusion {
            "success"             => Self::Success,
            "failure" | "timed_out" | "startup_failure" => Self::Failure,
            "cancelled"           => Self::Cancelled,
            _ => match status {
                "completed" => Self::Success,
                "in_progress" | "queued" | "waiting" => Self::Running,
                _ => Self::Pending,
            }
        }
    }
    pub fn from_gitlab(s: &str) -> Self {
        match s {
            "success" | "passed"  => Self::Success,
            "failed"              => Self::Failure,
            "running"             => Self::Running,
            "canceled" | "skipped" => Self::Cancelled,
            _                     => Self::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CiRun {
    pub id: String,
    pub name: String,
    pub status: CiStatus,
    pub url: String,
    pub branch: String,
    pub updated_ts: i64,
}

#[derive(Debug, Clone)]
pub struct ReleaseItem {
    pub tag_name: String,
    pub name: String,
    pub body: String,
    pub url: String,
    pub published_ts: i64,
    pub prerelease: bool,
}

/// Extract (owner, repo, host) from a git remote URL.
pub fn parse_remote(url: &str) -> Option<(String, String, String)> {
    let url = url.trim_end_matches(".git");
    // SSH: git@github.com:owner/repo
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        let (owner, repo) = path.split_once('/')?;
        return Some((owner.to_string(), repo.to_string(), host.to_string()));
    }
    // HTTPS: https://host/owner/repo  (or owner/group/project for GitLab)
    let url = url.trim_start_matches("https://").trim_start_matches("http://");
    let mut parts = url.splitn(4, '/');
    let host  = parts.next()?.to_string();
    let owner = parts.next()?.to_string();
    let repo  = parts.next()?.to_string();
    Some((owner, repo, host))
}
