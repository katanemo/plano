/// Extract AWS region from Bedrock base URL
///
/// Supports:
/// - Standard AWS regions: bedrock-runtime.us-east-1.amazonaws.com
/// - China regions: bedrock-runtime.cn-north-1.amazonaws.com.cn
/// - GovCloud regions: bedrock-runtime.us-gov-east-1.amazonaws.com
/// - URLs with ports: bedrock-runtime.us-east-1.amazonaws.com:443
/// - URLs with protocol: https://bedrock-runtime.us-east-1.amazonaws.com
pub fn extract_region_from_base_url(base_url: &str) -> Option<String> {
    let url = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
        .unwrap_or(base_url);

    let url = if let Some(colon_pos) = url.find(':') {
        if let Some(slash_pos) = url[colon_pos..].find('/') {
            let potential_port = &url[colon_pos + 1..colon_pos + slash_pos];
            if !potential_port.is_empty() && potential_port.chars().all(|c| c.is_ascii_digit()) {
                format!("{}{}", &url[..colon_pos], &url[colon_pos + slash_pos..])
            } else {
                url.to_string()
            }
        } else {
            let potential_port = &url[colon_pos + 1..];
            if !potential_port.is_empty() && potential_port.chars().all(|c| c.is_ascii_digit()) {
                url[..colon_pos].to_string()
            } else {
                url.to_string()
            }
        }
    } else {
        url.to_string()
    };

    if let Some(domain) = url.as_str().strip_prefix("bedrock-runtime.") {
        if let Some(region_and_domain) = domain.strip_suffix(".amazonaws.com.cn") {
            return Some(region_and_domain.to_string());
        }
        if let Some(region_and_domain) = domain.strip_suffix(".amazonaws.com") {
            return Some(region_and_domain.to_string());
        }
    }

    None
}

/// Get STS endpoint for a given region
///
/// AWS supports both:
/// - Regional endpoints: sts.<region>.amazonaws.com
/// - Global endpoint: sts.amazonaws.com (uses us-east-1)
///
/// This function returns the regional endpoint. For global endpoint, use "us-east-1".
///
/// Supports:
/// - Standard AWS regions: sts.us-east-1.amazonaws.com
/// - China regions: sts.cn-north-1.amazonaws.com.cn
/// - GovCloud regions: sts.us-gov-east-1.amazonaws.com
pub fn get_sts_endpoint(region: &str) -> String {
    if region.starts_with("cn-") {
        return format!("https://sts.{}.amazonaws.com.cn", region);
    }

    if region.starts_with("us-gov-") {
        return format!("https://sts.{}.amazonaws.com", region);
    }

    format!("https://sts.{}.amazonaws.com", region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_region_from_base_url() {
        assert_eq!(
            extract_region_from_base_url("bedrock-runtime.us-east-1.amazonaws.com"),
            Some("us-east-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("https://bedrock-runtime.us-west-2.amazonaws.com"),
            Some("us-west-2".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("http://bedrock-runtime.eu-west-1.amazonaws.com"),
            Some("eu-west-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("bedrock-runtime.us-east-1.amazonaws.com:443"),
            Some("us-east-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("https://bedrock-runtime.us-west-2.amazonaws.com:443"),
            Some("us-west-2".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("bedrock-runtime.cn-north-1.amazonaws.com.cn"),
            Some("cn-north-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url(
                "https://bedrock-runtime.cn-northwest-1.amazonaws.com.cn:443"
            ),
            Some("cn-northwest-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("bedrock-runtime.us-gov-east-1.amazonaws.com"),
            Some("us-gov-east-1".to_string())
        );
        assert_eq!(
            extract_region_from_base_url("https://bedrock-runtime.us-gov-west-1.amazonaws.com:443"),
            Some("us-gov-west-1".to_string())
        );
        assert_eq!(extract_region_from_base_url("invalid-url"), None);
        assert_eq!(
            extract_region_from_base_url("model.us-east-1.amazonaws.com"),
            None
        );
    }

    #[test]
    fn test_get_sts_endpoint() {
        assert_eq!(
            get_sts_endpoint("us-east-1"),
            "https://sts.us-east-1.amazonaws.com"
        );
        assert_eq!(
            get_sts_endpoint("us-west-2"),
            "https://sts.us-west-2.amazonaws.com"
        );
        assert_eq!(
            get_sts_endpoint("eu-west-1"),
            "https://sts.eu-west-1.amazonaws.com"
        );
        assert_eq!(
            get_sts_endpoint("cn-north-1"),
            "https://sts.cn-north-1.amazonaws.com.cn"
        );
        assert_eq!(
            get_sts_endpoint("cn-northwest-1"),
            "https://sts.cn-northwest-1.amazonaws.com.cn"
        );
        assert_eq!(
            get_sts_endpoint("us-gov-east-1"),
            "https://sts.us-gov-east-1.amazonaws.com"
        );
        assert_eq!(
            get_sts_endpoint("us-gov-west-1"),
            "https://sts.us-gov-west-1.amazonaws.com"
        );
    }
}
