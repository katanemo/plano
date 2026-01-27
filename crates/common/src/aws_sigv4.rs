use crate::errors::AwsError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::SystemTime;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

pub struct SigV4Params {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
    pub service: String,
    pub method: String,
    pub uri: String,
    pub query_string: String,
    pub headers: BTreeMap<String, String>,
    pub payload: Vec<u8>,
}

pub fn sign_request(params: SigV4Params) -> Result<(String, String, String), AwsError> {
    let now = SystemTime::now();
    let date_stamp = format_date(now);
    let amz_date = format_date_time(now);

    let (canonical_request, signed_headers) = create_canonical_request(&params, &amz_date)?;

    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, params.region, params.service
    );
    let string_to_sign = create_string_to_sign(&amz_date, &credential_scope, &canonical_request)?;

    let signature = calculate_signature(
        &params.secret_access_key,
        &date_stamp,
        &params.region,
        &params.service,
        &string_to_sign,
    )?;

    let authorization = create_authorization_header(
        &params.access_key_id,
        &credential_scope,
        &signed_headers,
        &signature,
    )?;

    Ok((authorization, amz_date, signature))
}

fn create_canonical_request(
    params: &SigV4Params,
    amz_date: &str,
) -> Result<(String, String), AwsError> {
    let method = &params.method;

    let canonical_uri = normalize_uri_path(&params.uri);

    let canonical_querystring = canonicalize_query_string(&params.query_string)?;

    let (canonical_headers, signed_headers) =
        canonicalize_headers(&params.headers, amz_date, &params.session_token)?;

    let payload_hash = if params.payload.is_empty() {
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()
    } else {
        hex::encode(Sha256::digest(&params.payload))
    };

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        canonical_uri,
        canonical_querystring,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    Ok((canonical_request, signed_headers))
}

fn normalize_uri_path(uri_path: &str) -> String {
    if uri_path.is_empty() {
        return "/".to_string();
    }

    let result = if uri_path.starts_with('/') {
        Cow::Borrowed(uri_path)
    } else {
        Cow::Owned(format!("/{uri_path}"))
    };

    if !(result.contains('.') || result.contains("//")) {
        return percent_encode_uri(&result);
    }

    let normalized = normalize_path_segment(&result);
    percent_encode_uri(&normalized)
}

fn normalize_path_segment(uri_path: &str) -> String {
    let number_of_slashes = uri_path.matches('/').count();
    let mut normalized: Vec<&str> = Vec::with_capacity(number_of_slashes + 1);

    for segment in uri_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                normalized.pop();
            }
            otherwise => normalized.push(otherwise),
        }
    }

    let mut result = normalized.join("/");

    if !result.starts_with('/') {
        result.insert(0, '/');
    }

    if ends_with_slash(uri_path) && !result.ends_with('/') {
        result.push('/');
    }

    result
}

fn ends_with_slash(uri_path: &str) -> bool {
    ["/", "/.", "/./", "/..", "/../"]
        .iter()
        .any(|s| uri_path.ends_with(s))
}

fn canonicalize_query_string(query_string: &str) -> Result<String, AwsError> {
    if query_string.is_empty() {
        return Ok(String::new());
    }

    let mut params: Vec<(String, String)> = Vec::new();
    for param in query_string.split('&') {
        if param.is_empty() {
            continue;
        }
        let parts: Vec<&str> = param.splitn(2, '=').collect();
        let key = percent_encode(parts[0]);
        let value = if parts.len() > 1 {
            percent_encode(parts[1])
        } else {
            String::new()
        };
        params.push((key, value));
    }

    params.sort_by(|a, b| match a.0.cmp(&b.0) {
        std::cmp::Ordering::Equal => a.1.cmp(&b.1),
        other => other,
    });

    let mut result = String::new();
    for (i, (key, value)) in params.iter().enumerate() {
        if i > 0 {
            result.push('&');
        }
        result.push_str(key);
        result.push('=');
        result.push_str(value);
    }

    Ok(result)
}

fn canonicalize_headers(
    headers: &BTreeMap<String, String>,
    amz_date: &str,
    session_token: &Option<String>,
) -> Result<(String, String), AwsError> {
    let mut canonical_headers = BTreeMap::new();

    let host = extract_host_from_headers(headers)?;
    canonical_headers.insert("host".to_string(), normalize_header_value(&host));
    canonical_headers.insert("x-amz-date".to_string(), amz_date.to_string());

    if let Some(ref token) = session_token {
        canonical_headers.insert(
            "x-amz-security-token".to_string(),
            normalize_header_value(token),
        );
    }

    for (key, value) in headers {
        let key_lower = key.to_lowercase();
        if key_lower != "host"
            && key_lower != "x-amz-date"
            && key_lower != "x-amz-security-token"
            && key_lower != "authorization"
        {
            let normalized_value = normalize_header_value(value);
            if let Some(existing) = canonical_headers.get_mut(&key_lower) {
                *existing = format!("{},{}", existing, normalized_value);
            } else {
                canonical_headers.insert(key_lower.clone(), normalized_value);
            }
        }
    }

    let mut canonical_headers_str = String::new();
    let mut signed_headers = Vec::new();
    for (key, value) in &canonical_headers {
        canonical_headers_str.push_str(&format!("{}:{}\n", key, value));
        signed_headers.push(key.clone());
    }
    signed_headers.sort();
    let signed_headers_str = signed_headers.join(";");

    Ok((canonical_headers_str, signed_headers_str))
}

fn normalize_header_value(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .trim()
        .to_string()
}

fn create_string_to_sign(
    amz_date: &str,
    credential_scope: &str,
    canonical_request: &str,
) -> Result<String, AwsError> {
    let algorithm = "AWS4-HMAC-SHA256";
    let canonical_request_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));

    Ok(format!(
        "{}\n{}\n{}\n{}",
        algorithm, amz_date, credential_scope, canonical_request_hash
    ))
}

fn calculate_signature(
    secret_key: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
    string_to_sign: &str,
) -> Result<String, AwsError> {
    let k_secret = format!("AWS4{}", secret_key);
    let mut mac = HmacSha256::new_from_slice(k_secret.as_bytes())
        .map_err(|e| AwsError::SigningError(format!("Failed to create HMAC: {}", e)))?;
    mac.update(date_stamp.as_bytes());
    let k_date = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_date)
        .map_err(|e| AwsError::SigningError(format!("Failed to create HMAC: {}", e)))?;
    mac.update(region.as_bytes());
    let k_region = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_region)
        .map_err(|e| AwsError::SigningError(format!("Failed to create HMAC: {}", e)))?;
    mac.update(service.as_bytes());
    let k_service = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_service)
        .map_err(|e| AwsError::SigningError(format!("Failed to create HMAC: {}", e)))?;
    mac.update(b"aws4_request");
    let k_signing = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_signing)
        .map_err(|e| AwsError::SigningError(format!("Failed to create HMAC: {}", e)))?;
    mac.update(string_to_sign.as_bytes());
    let signature = mac.finalize().into_bytes();

    Ok(hex::encode(signature))
}

fn create_authorization_header(
    access_key_id: &str,
    credential_scope: &str,
    signed_headers: &str,
    signature: &str,
) -> Result<String, AwsError> {
    let algorithm = "AWS4-HMAC-SHA256";

    Ok(format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        algorithm, access_key_id, credential_scope, signed_headers, signature
    ))
}

fn extract_host_from_headers(headers: &BTreeMap<String, String>) -> Result<String, AwsError> {
    for (key, value) in headers {
        if key.to_lowercase() == "host" {
            return Ok(value.trim().to_string());
        }
    }

    Err(AwsError::SigningError(
        "Host header not found in request headers".to_string(),
    ))
}

fn percent_encode_uri(uri: &str) -> String {
    let mut encoded = String::new();
    for byte in uri.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

fn percent_encode(s: &str) -> String {
    let mut encoded = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

fn format_date(time: SystemTime) -> String {
    let time = OffsetDateTime::from(time);
    format!(
        "{:04}{:02}{:02}",
        time.year(),
        u8::from(time.month()),
        time.day()
    )
}

pub fn format_date_time(time: SystemTime) -> String {
    let time = OffsetDateTime::from(time);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_encode_uri() {
        assert_eq!(percent_encode_uri("/path/to/resource"), "/path/to/resource");
        assert_eq!(
            percent_encode_uri("/path with spaces"),
            "/path%20with%20spaces"
        );
    }

    #[test]
    fn test_canonicalize_query_string() {
        assert_eq!(canonicalize_query_string("").unwrap(), "");
        assert_eq!(canonicalize_query_string("b=2&a=1").unwrap(), "a=1&b=2");
        assert_eq!(canonicalize_query_string("a=1&a=2").unwrap(), "a=1&a=2");
    }

    #[test]
    fn test_normalize_header_value() {
        assert_eq!(normalize_header_value("  value  "), "value");
        assert_eq!(
            normalize_header_value("value   with   spaces"),
            "value with spaces"
        );
    }

    #[test]
    fn test_normalize_uri_path() {
        assert_eq!(normalize_uri_path(""), "/");
        assert_eq!(normalize_uri_path("/"), "/");
        assert_eq!(normalize_uri_path("/foo"), "/foo");
        assert_eq!(normalize_uri_path("foo"), "/foo");
        assert_eq!(normalize_uri_path("/./"), "/");
        assert_eq!(normalize_uri_path("/../"), "/");
        assert_eq!(normalize_uri_path("/foo/bar/.."), "/foo/");
        assert_eq!(normalize_uri_path("//foo//"), "/foo/");
    }
}
