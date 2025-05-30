use std::{env, fs, process::exit};

/// Run `rustc -O check_commit_message.rs -o check_commit_message` to generate a git hook from this rs file.
fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or(".git/COMMIT_EDITMSG".to_string());

    let msg = fs::read_to_string(path)
        .expect("❌ Failed to read commit message")
        .trim()
        .to_string();

    let allowed_types = [
        "feat", "fix", "chore", "docs", "refactor", "test", "style", "perf", "build",
    ];

    let parts: Vec<&str> = msg.splitn(2, ':').collect();
    if parts.len() != 2 {
        fail(&msg);
    }

    let (prefix, description) = (parts[0].trim(), parts[1].trim());
    if description.is_empty() {
        fail(&msg);
    }

    // Handle optional scope in prefix, e.g., feat(wallet)
    let type_valid = if let Some(start_paren) = prefix.find('(') {
        if let Some(end_paren) = prefix.find(')') {
            if end_paren > start_paren {
                let type_part = &prefix[..start_paren];
                allowed_types.contains(&type_part)
            } else {
                false
            }
        } else {
            false
        }
    } else {
        allowed_types.contains(&prefix)
    };

    if !type_valid {
        fail(&msg);
    }

    println!("✅ Commit message is valid: \"{msg}\"");
    exit(0);
}

fn fail(msg: &str) -> ! {
    println!("❌ Invalid commit message:\n\n\"{msg}\"\n");
    println!("Expected format: type(scope?): description");
    println!("Allowed types: feat, fix, chore, docs, refactor, test, style, perf, build");
    println!("Example: fix(wallet): handle gas estimation issue");
    exit(1);
}
