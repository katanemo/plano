use anyhow::{bail, Result};

pub async fn run(
    include_spans: Option<&str>,
    exclude_spans: Option<&str>,
    where_filters: &[String],
    since: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    // TODO: Connect to trace listener via gRPC and tail traces
    // For now, print a placeholder

    println!("Tailing traces...");
    if let Some(inc) = include_spans {
        println!("  include: {inc}");
    }
    if let Some(exc) = exclude_spans {
        println!("  exclude: {exc}");
    }
    for w in where_filters {
        println!("  where: {w}");
    }
    if let Some(s) = since {
        println!("  since: {s}");
    }

    // The full implementation will:
    // 1. Connect to the gRPC trace query service
    // 2. Fetch recent traces
    // 3. Apply filters
    // 4. Render matching traces

    bail!("Trace tail is not yet fully implemented. The gRPC trace query service needs to be running.")
}

/// Parse a "since" string like "10s", "5m", "1h", "7d" into seconds.
pub fn parse_since_seconds(since: &str) -> Option<u64> {
    let since = since.trim();
    if since.is_empty() {
        return None;
    }

    let (num_str, unit) = since.split_at(since.len() - 1);
    let num: u64 = num_str.parse().ok()?;

    match unit {
        "s" => Some(num),
        "m" => Some(num * 60),
        "h" => Some(num * 3600),
        "d" => Some(num * 86400),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_since_seconds() {
        assert_eq!(parse_since_seconds("10s"), Some(10));
        assert_eq!(parse_since_seconds("5m"), Some(300));
        assert_eq!(parse_since_seconds("1h"), Some(3600));
        assert_eq!(parse_since_seconds("7d"), Some(604800));
        assert_eq!(parse_since_seconds(""), None);
        assert_eq!(parse_since_seconds("abc"), None);
    }
}
