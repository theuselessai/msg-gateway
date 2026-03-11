//! TTY-aware output formatting.
//!
//! When stdout is a TTY (interactive), output is human-friendly.
//! When piped or `--json` is set, output is JSON (JSONL for streams).

use serde::Serialize;

/// Print a single value — JSON when json_output, human-friendly otherwise.
pub fn print_result<T: Serialize + std::fmt::Debug>(value: &T, json_output: bool) {
    if json_output {
        // Compact JSON to stdout
        println!(
            "{}",
            serde_json::to_string(value).expect("Failed to serialize output")
        );
    } else {
        // Pretty debug for now — can be enhanced with table formatting later
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("Failed to serialize output")
        );
    }
}

/// Print a single JSONL line (for streaming output like `plit listen`).
pub fn print_jsonl<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string(value).expect("Failed to serialize output")
    );
}

/// Print a status/info message to stderr (never interferes with stdout data).
pub fn status(msg: &str) {
    eprintln!("{}", msg);
}

/// Print an error message to stderr.
pub fn error(msg: &str) {
    eprintln!("error: {}", msg);
}
