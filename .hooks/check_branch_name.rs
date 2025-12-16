use std::fs;
use std::process::{exit, Command};

/// Run `rustc -O check_branch_name.rs -o check_branch_name` to generate a git hook from this rs file.
fn main() {
    // 1. Skip if a rebase is in progress
    if fs::metadata(".git/rebase-merge").is_ok() || fs::metadata(".git/rebase-apply").is_ok() {
        println!("In the middle of a rebase, skipping branch name check");
        exit(0);
    }

    // 2. Check if we're pushing tags - if so, skip branch name validation
    // When pushing tags, HEAD is typically detached or we can check for tag refs
    // Note: We intentionally ignore execution errors here (if git is unavailable or command fails)
    // to proceed to step 3, which checks for detached HEAD state (common when pushing tags)
    let tag_check = Command::new("git")
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output();
    
    if let Ok(tag_output) = tag_check {
        if tag_output.status.success() {
            let tag_name = String::from_utf8_lossy(&tag_output.stdout).trim().to_string();
            println!("Pushing tag '{}', skipping branch name check", tag_name);
            exit(0);
        }
    }

    // 3. Also check if we're in detached HEAD state (common when pushing tags)
    let branch_check = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output();
    
    let branch_output = match branch_check {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Failed to execute git: {}", e);
            exit(1);
        }
    };

    if !branch_output.status.success() {
        // No branch name means we're in detached HEAD state (e.g., pushing a tag)
        // Skip branch name check in this case
        println!("Detached HEAD detected (likely pushing a tag), skipping branch name check");
        exit(0);
    }

    // 4. Valid prefixes
    let valid_prefixes = [
        "feat", "fix", "chore", "docs", "refactor", "test", "style", "perf", "build",
    ];

    let branch_name = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    // 5. Build the pattern and validate
    let is_valid = valid_prefixes
        .iter()
        .any(|prefix| branch_name.starts_with(&format!("{}/", prefix)));

    if !is_valid {
        eprintln!(
            "❌ Error: Branch name '{}' is invalid.\n\
             It must start with one of: ({})/",
            branch_name,
            valid_prefixes.join(", ")
        );
        exit(1);
    }

    // 6. All good
    exit(0);
}
