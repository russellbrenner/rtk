//! Runs curl and auto-compresses JSON responses.

use crate::core::tracking;
use crate::core::utils::{exit_code_from_output, resolved_command, truncate};
use crate::json_cmd;
use anyhow::{Context, Result};

/// Not using run_filtered: on failure, curl can return HTML error pages (404, 500)
/// that the JSON schema filter would mangle. The early exit skips filtering entirely.
pub fn run(args: &[String], verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    let mut cmd = resolved_command("curl");
    cmd.arg("-s"); // Silent mode (no progress bar)

    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: curl -s {}", args.join(" "));
    }

    let output = cmd.output().context("Failed to run curl")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Early exit: don't feed HTTP error bodies (HTML 404 etc.) through JSON schema filter
    if !output.status.success() {
        let msg = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        eprintln!("FAILED: curl {}", msg);
        return Ok(exit_code_from_output(&output, "curl"));
    }

    let raw = stdout.to_string();

    // Auto-detect JSON and pipe through filter
    let filtered = filter_curl_output(&stdout);
    println!("{}", filtered);

    timer.track(
        &format!("curl {}", args.join(" ")),
        &format!("rtk curl {}", args.join(" ")),
        &raw,
        &filtered,
    );

    Ok(0)
}

fn filter_curl_output(output: &str) -> String {
    let trimmed = output.trim();

    // JSON output: pass through unchanged to preserve validity for piping (#1015)
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && (trimmed.ends_with('}') || trimmed.ends_with(']'))
    {
        return trimmed.to_string();
    }

    // Non-JSON: truncate long output
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() > 50 {
        let mut result: Vec<&str> = lines[..50].to_vec();
        result.push("");
        let msg = format!(
            "... ({} more lines, {} bytes total)",
            lines.len() - 50,
            trimmed.len()
        );
        return format!("{}\n{}", result.join("\n"), msg);
    }

    // Short non-JSON output: truncate long lines
    lines
        .iter()
        .map(|l| truncate(l, 300))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_curl_json_preserves_valid_json() {
        // curl output must remain valid JSON for downstream parsers (#1015)
        let output = r#"{"name": "test", "count": 42, "items": [1, 2, 3]}"#;
        let result = filter_curl_output(output);
        assert!(result.contains("\"name\""));
        assert!(result.contains("42"));
        // Must be parseable as JSON
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok(),
            "curl output must be valid JSON: {}", result);
    }

    #[test]
    fn test_filter_curl_non_json() {
        let output = "Hello, World!\nThis is plain text.";
        let result = filter_curl_output(output);
        assert!(result.contains("Hello, World!"));
        assert!(result.contains("plain text"));
    }

    #[test]
    fn test_filter_curl_json_small_returns_original() {
        let output = r#"{"r2Ready":true,"status":"ok"}"#;
        let result = filter_curl_output(output);
        assert_eq!(result.trim(), output.trim());
    }

    #[test]
    fn test_filter_curl_long_output() {
        let lines: Vec<String> = (0..80).map(|i| format!("Line {}", i)).collect();
        let output = lines.join("\n");
        let result = filter_curl_output(&output);
        assert!(result.contains("Line 0"));
        assert!(result.contains("Line 49"));
        assert!(result.contains("more lines"));
    }
}
