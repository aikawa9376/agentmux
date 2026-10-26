//! Read-only Git context for the TUI. Neither drawing nor the observer waits on Git.
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    process::{Command, Output, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const GIT_TIMEOUT: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    pub root: String,
    pub branch: Option<String>,
}

struct Entry {
    project: Option<Project>,
    checked: Instant,
}

pub(crate) struct ProjectCache {
    entries: HashMap<String, Entry>,
    pending: HashSet<String>,
    jobs: SyncSender<Vec<String>>,
    results: Receiver<(String, Option<Project>)>,
}

impl ProjectCache {
    pub fn new() -> Self {
        let (jobs, requests) = mpsc::sync_channel::<Vec<String>>(1);
        let (results, responses) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(paths) = requests.recv() {
                for cwd in paths {
                    let project = discover(&cwd);
                    if results.send((cwd, project)).is_err() {
                        return;
                    }
                }
            }
        });
        Self {
            entries: HashMap::new(),
            pending: HashSet::new(),
            jobs,
            results: responses,
        }
    }

    pub fn refresh(&mut self, paths: Vec<String>) {
        let active: HashSet<_> = paths.iter().cloned().collect();
        while let Ok((cwd, project)) = self.results.try_recv() {
            self.pending.remove(&cwd);
            if active.contains(&cwd) {
                self.entries.insert(
                    cwd,
                    Entry {
                        project,
                        checked: Instant::now(),
                    },
                );
            }
        }
        self.entries.retain(|cwd, _| active.contains(cwd));
        let mut queued = HashSet::new();
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|cwd| {
                !cwd.is_empty()
                    && !self.pending.contains(cwd)
                    && self
                        .entries
                        .get(cwd)
                        .is_none_or(|e| e.checked.elapsed() >= REFRESH_INTERVAL)
                    && queued.insert(cwd.clone())
            })
            .collect();
        if !paths.is_empty() && self.jobs.try_send(paths.clone()).is_ok() {
            self.pending.extend(paths);
        }
    }

    pub fn get(&self, cwd: &str) -> Option<&Project> {
        self.entries.get(cwd)?.project.as_ref()
    }
}

fn discover(cwd: &str) -> Option<Project> {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let branch = git(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]).or_else(|| {
        git(cwd, &["rev-parse", "--short", "HEAD"]).map(|sha| format!("detached@{sha}"))
    });
    Some(Project { root, branch })
}

fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A caller's shell may have selected a different repository explicitly.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    let output = command_output(command, GIT_TIMEOUT)?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim_end_matches(['\r', '\n']);
    (!text.is_empty()).then(|| text.to_owned())
}

fn command_output(mut command: Command, timeout: Duration) -> Option<Output> {
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub(crate) fn directory_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, time::SystemTime};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "agentmux-project-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.0)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolves_nested_unborn_detached_and_worktree_contexts() {
        let fixture = Fixture::new();
        let cwd = fixture.0.to_str().unwrap();
        assert_eq!(discover(cwd), None);
        fixture.git(&["init", "--initial-branch=main"]);
        fs::create_dir(fixture.0.join("nested")).unwrap();
        let nested = fixture.0.join("nested");
        let context = discover(nested.to_str().unwrap()).unwrap();
        assert_eq!(context.root, cwd);
        assert_eq!(context.branch.as_deref(), Some("main"));
        fixture.git(&["commit", "--allow-empty", "-m", "fixture"]);
        let worktree = fixture.0.join("checkout");
        fixture.git(&[
            "worktree",
            "add",
            "-b",
            "feature/sidebar",
            worktree.to_str().unwrap(),
        ]);
        let context = discover(worktree.to_str().unwrap()).unwrap();
        assert_eq!(context.root, worktree.to_str().unwrap());
        assert_eq!(context.branch.as_deref(), Some("feature/sidebar"));
        fixture.git(&["checkout", "--detach"]);
        assert!(
            discover(cwd)
                .unwrap()
                .branch
                .unwrap()
                .starts_with("detached@")
        );
        assert_eq!(discover(fixture.0.join("missing").to_str().unwrap()), None);
    }

    #[test]
    fn cache_deduplicates_refreshes_branches_and_discards_removed_paths() {
        let fixture = Fixture::new();
        fixture.git(&["init", "--initial-branch=main"]);
        let cwd = fixture.0.to_str().unwrap().to_owned();
        let paths = vec![cwd.clone(), cwd.clone()];
        let mut cache = ProjectCache::new();
        cache.refresh(paths.clone());
        assert_eq!(cache.pending.len(), 1);
        let deadline = Instant::now() + Duration::from_secs(3);
        while cache.get(&cwd).is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
            cache.refresh(paths.clone());
        }
        assert_eq!(cache.get(&cwd).unwrap().branch.as_deref(), Some("main"));
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/renamed"]);
        cache.entries.get_mut(&cwd).unwrap().checked = Instant::now() - REFRESH_INTERVAL;
        cache.refresh(paths.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        while cache.get(&cwd).unwrap().branch.as_deref() != Some("renamed")
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
            cache.refresh(paths.clone());
        }
        assert_eq!(cache.get(&cwd).unwrap().branch.as_deref(), Some("renamed"));
        cache.refresh(vec![]);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn slow_commands_are_killed_with_a_bounded_wait() {
        let mut command = Command::new("sleep");
        command.arg("2");
        let started = Instant::now();
        assert!(command_output(command, Duration::from_millis(30)).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
