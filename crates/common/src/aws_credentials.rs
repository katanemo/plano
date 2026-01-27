use crate::aws_sigv4::{sign_request, SigV4Params};
use crate::aws_utils::get_sts_endpoint;
use crate::errors::AwsError;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use time::format_description;
use time::PrimitiveDateTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    #[serde(with = "serde_system_time")]
    pub expiration: SystemTime,
}

mod serde_system_time {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time.duration_since(UNIX_EPOCH).unwrap();
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

pub fn get_credentials_from_config(
    config: &crate::configuration::AwsCredentialsConfig,
) -> Result<(String, String, Option<String>), AwsError> {
    let access_key_id = config
        .access_key_id
        .as_ref()
        .ok_or_else(|| {
            AwsError::CredentialError("AWS_ACCESS_KEY_ID not found in configuration".to_string())
        })?
        .clone();

    let secret_access_key = config
        .secret_access_key
        .as_ref()
        .ok_or_else(|| {
            AwsError::CredentialError(
                "AWS_SECRET_ACCESS_KEY not found in configuration".to_string(),
            )
        })?
        .clone();

    Ok((
        access_key_id,
        secret_access_key,
        config.session_token.clone(),
    ))
}

pub fn build_sts_assume_role_request(role_arn: &str, role_session_name: &str) -> String {
    use urlencoding::encode;
    format!(
        "Action=AssumeRole&RoleArn={}&RoleSessionName={}&Version=2011-06-15",
        encode(role_arn),
        encode(role_session_name)
    )
}

type StsRequestResult = (Vec<(String, String)>, Vec<u8>, String, String);

pub fn build_sts_request(
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
    role_arn: &str,
    region: &str,
    request_id: Option<&str>,
) -> Result<StsRequestResult, AwsError> {
    let sts_endpoint = get_sts_endpoint(region);
    let sts_host = sts_endpoint
        .strip_prefix("https://")
        .ok_or_else(|| AwsError::ConfigError("Invalid STS endpoint".to_string()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AwsError::SigningError(format!("Failed to get current time: {}", e)))?
        .as_secs();

    let role_session_name = if let Some(req_id) = request_id {
        format!("PlanoSession-{}-{}", timestamp, req_id)
    } else {
        format!("PlanoSession-{}", timestamp)
    };
    let request_body = build_sts_assume_role_request(role_arn, &role_session_name);
    let body_bytes = request_body.as_bytes().to_vec();

    let now = SystemTime::now();
    let amz_date = crate::aws_sigv4::format_date_time(now);

    let mut headers = BTreeMap::new();
    headers.insert("host".to_string(), sts_host.to_string());
    headers.insert("x-amz-date".to_string(), amz_date.clone());

    headers.insert(
        "content-type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    );
    if let Some(token) = session_token {
        headers.insert("x-amz-security-token".to_string(), token.to_string());
    }

    let (authorization, _amz_date, _signature) = sign_request(SigV4Params {
        access_key_id: access_key_id.to_string(),
        secret_access_key: secret_access_key.to_string(),
        session_token: session_token.map(|s| s.to_string()),
        region: region.to_string(),
        service: "sts".to_string(),
        method: "POST".to_string(),
        uri: "/".to_string(),
        query_string: String::new(),
        headers: headers.clone(),
        payload: body_bytes.clone(),
    })?;

    let mut http_headers = Vec::new();
    http_headers.push(("x-amz-date".to_string(), amz_date.clone()));
    http_headers.push(("authorization".to_string(), authorization));
    if let Some(token) = session_token {
        http_headers.push(("x-amz-security-token".to_string(), token.to_string()));
    }
    http_headers.push((
        "content-type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    ));
    http_headers.push(("content-length".to_string(), body_bytes.len().to_string()));

    Ok((http_headers, body_bytes, sts_endpoint, role_session_name))
}

pub fn parse_sts_response(xml_body: &[u8]) -> Result<AwsCredentials, AwsError> {
    let xml_str = String::from_utf8(xml_body.to_vec())
        .map_err(|e| AwsError::StsError(format!("Failed to parse STS response as UTF-8: {}", e)))?;

    let access_key_id = extract_xml_value(&xml_str, "AccessKeyId")
        .ok_or_else(|| AwsError::StsError("AccessKeyId not found in STS response".to_string()))?;

    let secret_access_key = extract_xml_value(&xml_str, "SecretAccessKey").ok_or_else(|| {
        AwsError::StsError("SecretAccessKey not found in STS response".to_string())
    })?;

    let session_token = extract_xml_value(&xml_str, "SessionToken")
        .ok_or_else(|| AwsError::StsError("SessionToken not found in STS response".to_string()))?;

    let expiration_str = extract_xml_value(&xml_str, "Expiration")
        .ok_or_else(|| AwsError::StsError("Expiration not found in STS response".to_string()))?;

    let expiration = parse_iso8601_datetime(&expiration_str)
        .map_err(|e| AwsError::StsError(format!("Failed to parse expiration: {}", e)))?;

    Ok(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
        expiration,
    })
}

// NOTE: STS XML format is stable and flat; full XML parsing is intentionally avoided.
// This function assumes:
// - No XML namespaces
// - No formatting changes (whitespace, line breaks)
// - Simple flat structure with direct tag content
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{}>", tag);
    let close_tag = format!("</{}>", tag);

    xml.find(&open_tag).and_then(|start| {
        let content_start = start + open_tag.len();
        xml[content_start..]
            .find(&close_tag)
            .map(|end| xml[content_start..content_start + end].trim().to_string())
    })
}

fn parse_iso8601_datetime(datetime_str: &str) -> Result<SystemTime, String> {
    let datetime_str = datetime_str.trim();

    const ISO8601_FORMAT: &str = "[year]-[month]-[day]T[hour]:[minute]:[second]Z";

    let format_desc = format_description::parse(ISO8601_FORMAT)
        .map_err(|e| format!("Failed to parse format description: {}", e))?;

    let date_time: SystemTime = PrimitiveDateTime::parse(datetime_str, &format_desc)
        .map_err(|e| format!("Failed to parse datetime: {}", e))?
        .assume_utc()
        .into();

    Ok(date_time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xml_value() {
        let xml = "<AccessKeyId>ASIA123</AccessKeyId>";
        assert_eq!(
            extract_xml_value(xml, "AccessKeyId"),
            Some("ASIA123".to_string())
        );
    }
}
