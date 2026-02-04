use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub initial_commit: Option<String>,
    pub modified_files: HashSet<String>,
    pub captures: Vec<CaptureRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub timestamp: DateTime<Utc>,
    pub commit: Option<String>,
    pub files_changed: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: generate_session_id(),
            started_at: Utc::now(),
            ended_at: None,
            initial_commit: None,
            modified_files: HashSet::new(),
            captures: Vec::new(),
        }
    }
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    pub fn duration(&self) -> chrono::Duration {
        let end = self.ended_at.unwrap_or_else(Utc::now);
        end - self.started_at
    }
}

fn generate_session_id() -> String {
    let now = Utc::now();
    format!("session-{}", now.format("%Y%m%d-%H%M%S"))
}

pub async fn start_session(root: &Path) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let session_path = charter_dir.join("session.json");

    if session_path.exists() {
        let content = fs::read_to_string(&session_path).await?;
        if let Ok(existing) = serde_json::from_str::<Session>(&content) {
            if existing.is_active() {
                println!("Session already active: {}", existing.id);
                println!(
                    "Started: {}",
                    existing.started_at.format("%Y-%m-%d %H:%M:%S UTC")
                );
                println!("Duration: {}", format_duration(existing.duration()));
                println!();
                println!("Use 'charter session end' to end the current session first.");
                return Ok(());
            }
        }
    }

    let mut session = Session::new();

    let git_commit = crate::git::get_git_info(root).await.ok();
    session.initial_commit = git_commit.map(|g| g.commit_short);

    let content = serde_json::to_string_pretty(&session)?;
    fs::write(&session_path, content).await?;

    println!("Session started: {}", session.id);
    if let Some(ref commit) = session.initial_commit {
        println!("Initial commit: {}", commit);
    }
    println!(
        "Started at: {}",
        session.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    Ok(())
}

pub async fn end_session(root: &Path) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let session_path = charter_dir.join("session.json");

    if !session_path.exists() {
        println!("No active session found.");
        println!("Use 'charter session start' to begin a new session.");
        return Ok(());
    }

    let content = fs::read_to_string(&session_path).await?;
    let mut session: Session = serde_json::from_str(&content)?;

    if !session.is_active() {
        println!("Session {} is already ended.", session.id);
        return Ok(());
    }

    session.ended_at = Some(Utc::now());

    let final_commit = crate::git::get_git_info(root).await.ok();

    println!("Session ended: {}", session.id);
    println!("Duration: {}", format_duration(session.duration()));
    println!();

    if !session.modified_files.is_empty() {
        println!(
            "Files modified this session ({}):",
            session.modified_files.len()
        );
        for file in session.modified_files.iter().take(20) {
            println!("  {}", file);
        }
        if session.modified_files.len() > 20 {
            println!("  ... and {} more", session.modified_files.len() - 20);
        }
        println!();
    }

    if !session.captures.is_empty() {
        println!("Captures performed: {}", session.captures.len());
    }

    match (
        &session.initial_commit,
        final_commit.as_ref().map(|g| &g.commit_short),
    ) {
        (Some(initial), Some(final_c)) if initial != final_c => {
            println!("Commits: {} → {}", initial, final_c);
        }
        (Some(initial), _) => {
            println!("Starting commit: {}", initial);
        }
        _ => {}
    }

    let history_dir = charter_dir.join("sessions");
    fs::create_dir_all(&history_dir).await?;

    let history_path = history_dir.join(format!("{}.json", session.id));
    let content = serde_json::to_string_pretty(&session)?;
    fs::write(&history_path, content).await?;

    fs::remove_file(&session_path).await?;

    println!();
    println!("Session archived to: sessions/{}.json", session.id);

    Ok(())
}

pub async fn session_status(root: &Path) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let session_path = charter_dir.join("session.json");

    if !session_path.exists() {
        println!("No active session.");
        println!();

        let history_dir = charter_dir.join("sessions");
        if history_dir.exists() {
            let mut entries = fs::read_dir(&history_dir).await?;
            let mut sessions = Vec::new();

            while let Some(entry) = entries.next_entry().await? {
                if entry.path().extension().is_some_and(|e| e == "json") {
                    sessions.push(entry.file_name().to_string_lossy().to_string());
                }
            }

            if !sessions.is_empty() {
                sessions.sort();
                sessions.reverse();
                println!("Recent sessions:");
                for session in sessions.iter().take(5) {
                    println!("  {}", session.trim_end_matches(".json"));
                }
            }
        }

        println!();
        println!("Use 'charter session start' to begin a new session.");
        return Ok(());
    }

    let content = fs::read_to_string(&session_path).await?;
    let session: Session = serde_json::from_str(&content)?;

    println!("Active session: {}", session.id);
    println!(
        "Started: {}",
        session.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("Duration: {}", format_duration(session.duration()));

    if let Some(ref commit) = session.initial_commit {
        println!("Initial commit: {}", commit);

        if let Ok(git) = crate::git::get_git_info(root).await {
            if git.commit_short != *commit {
                println!("Current commit: {}", git.commit_short);
            }
        }
    }

    println!();

    if !session.modified_files.is_empty() {
        println!("Modified files ({}):", session.modified_files.len());
        for file in session.modified_files.iter().take(10) {
            println!("  {}", file);
        }
        if session.modified_files.len() > 10 {
            println!("  ... and {} more", session.modified_files.len() - 10);
        }
        println!();
    }

    if !session.captures.is_empty() {
        println!("Captures: {}", session.captures.len());
        if let Some(last) = session.captures.last() {
            println!(
                "Last capture: {} ({} files changed)",
                last.timestamp.format("%H:%M:%S"),
                last.files_changed
            );
        }
    }

    println!();
    println!("Use 'charter session end' to end this session.");

    Ok(())
}

#[allow(dead_code)]
pub async fn update_session_on_capture(
    root: &Path,
    files_changed: usize,
    commit: Option<&str>,
) -> Result<()> {
    let session_path = root.join(".charter/session.json");

    if !session_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&session_path).await?;
    let mut session: Session = serde_json::from_str(&content)?;

    if !session.is_active() {
        return Ok(());
    }

    session.captures.push(CaptureRecord {
        timestamp: Utc::now(),
        commit: commit.map(|s| s.to_string()),
        files_changed,
    });

    let content = serde_json::to_string_pretty(&session)?;
    fs::write(&session_path, content).await?;

    Ok(())
}

#[allow(dead_code)]
pub async fn track_modified_file(root: &Path, file_path: &str) -> Result<()> {
    let session_path = root.join(".charter/session.json");

    if !session_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&session_path).await?;
    let mut session: Session = serde_json::from_str(&content)?;

    if !session.is_active() {
        return Ok(());
    }

    session.modified_files.insert(file_path.to_string());

    let content = serde_json::to_string_pretty(&session)?;
    fs::write(&session_path, content).await?;

    Ok(())
}

fn format_duration(duration: chrono::Duration) -> String {
    let total_seconds = duration.num_seconds();

    if total_seconds < 60 {
        format!("{}s", total_seconds)
    } else if total_seconds < 3600 {
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{}m {}s", minutes, seconds)
    } else {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        format!("{}h {}m", hours, minutes)
    }
}
