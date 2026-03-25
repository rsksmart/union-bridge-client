use anyhow::Result;
use reqwest::Request;
use serde_json;
use std::process::Command;

/// Prompts the user for confirmation before executing a remote operation.
/// Shows the specific command or endpoint that will be executed.
pub fn confirm_operation(description: &str) -> Result<bool> {
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
pub fn command_to_string(cmd: &Command) -> String {
    let program = cmd.get_program().to_string_lossy();
    let args: Vec<String> = cmd.get_args().map(|arg| arg.to_string_lossy().to_string()).collect();

    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

/// Converts an HTTP Request to a string representation for display
pub fn request_to_string(request: &Request) -> String {
    let method = request.method();
    let url = request.url();

    let mut description = format!("{} {}", method, url);

    // try to extract and display the body if it exists
    if let Some(body) = request.body() {
        if let Some(bytes) = body.as_bytes() {
            if let Ok(json_str) = std::str::from_utf8(bytes) {
                // try to pretty-print the JSON
                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&json_value) {
                        description.push_str(&format!("\nPayload:\n{}", pretty));
                    }
                } else {
                    description.push_str(&format!("\nPayload: {}", json_str));
                }
            }
        }
    }

    description
}
