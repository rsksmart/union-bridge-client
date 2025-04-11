use std::fs;
use std::process::{exit, Command};

/// Run `rustc -O check_branch_name.rs -o check_branch_name` to generate a git hook from this rs file.
fn main() {
    // 1. Skip if a rebase is in progress
    if fs::metadata(".git/rebase-merge").is_ok() || fs::metadata(".git/rebase-apply").is_ok() {
        println!("In the middle of a rebase, skipping branch name check");
        exit(0);
    }

    // 2. Valid prefixes
    let valid_prefixes = [
        "feat", "fix", "chore", "docs", "refactor", "test", "style", "perf", "build",
    ];

    // 3. Get current branch name using `git symbolic-ref --short HEAD`
    let branch_output = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .expect("Failed to execute git");

    if !branch_output.status.success() {
        eprintln!("Failed to get current branch name");
        exit(1);
    }

    let branch_name = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    // 4. Build the pattern and validate
    let is_valid = valid_prefixes
        .iter()
        .any(|prefix| branch_name.starts_with(&format!("{}/", prefix)));

    if !is_valid {
        eprintln!(
            "❌ Error: Branch name '{}' is invalid.\n\
             It must start with one of: ({})/REM",
            branch_name,
            valid_prefixes.join(", ")
        );
        exit(1);
    }

    // 5. All good
    exit(0);
}
