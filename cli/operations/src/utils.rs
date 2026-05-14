use std::process::Command;

use anyhow::{bail, Context, Result};
use reqwest::Request;

/// Prompts the user for confirmation before executing a remote operation.
/// Shows the specific command or endpoint that will be executed.
pub(crate) fn confirm_operation(description: &str) -> Result<bool> {
    println!("\n⚠️  REMOTE OPERATION ⚠️");
    println!("{}", description);
    print!("\nProceed? (yes/no): ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    let answer = input.trim().to_lowercase();
    Ok(answer == "yes" || answer == "y")
}

/// Converts a Command to a string representation for display
pub(crate) fn command_to_string(cmd: &Command) -> String {
    let program = cmd.get_program().to_string_lossy();
    let args: Vec<String> = cmd.get_args().map(|arg| arg.to_string_lossy().to_string()).collect();

    if args.is_empty() { program.to_string() } else { format!("{} {}", program, args.join(" ")) }
}

/// Runs a bitcoin-wallet CLI command and returns its stdout.
pub(crate) fn run_wallet_command(args: &[&str]) -> Result<String> {
    let wallet_script = "./cli-bitcoin-wallet.sh";
    let mut cmd = Command::new(wallet_script);
    cmd.args(args);

    let rendered_args = args.join(" ");
    println!("Running: {} {}", wallet_script, rendered_args);

    let output = cmd.output().context("failed to execute cli-bitcoin-wallet.sh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "wallet command failed with status {}:\nstdout: {}\nstderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Converts an HTTP Request to a string representation for display
pub(crate) fn request_to_string(request: &Request) -> String {
    let method = request.method();
    let url = request.url();

    let mut description = format!("{} {}", method, url);

    // try to extract and display the body if it exists
    if let Some(body) = request.body()
        && let Some(bytes) = body.as_bytes()
        && let Ok(json_str) = std::str::from_utf8(bytes)
    {
        // try to pretty-print the JSON
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Ok(pretty) = serde_json::to_string_pretty(&json_value) {
                description.push_str(&format!("\nPayload:\n{}", pretty));
            }
        } else {
            description.push_str(&format!("\nPayload: {}", json_str));
        }
    }

    description
}
