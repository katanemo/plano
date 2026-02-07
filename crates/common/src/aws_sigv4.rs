use crate::errors::AwsError;
use std::collections::BTreeMap;

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

#[cfg(feature = "aws-sigv4")]
pub fn sign_request(params: SigV4Params) -> Result<(String, String), AwsError> {
    use aws_credential_types::Credentials;
    use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
    use aws_sigv4::sign::v4;
    use aws_smithy_runtime_api::client::identity::Identity;
    use std::time::SystemTime;

    let credentials = Credentials::new(
        &params.access_key_id,
        &params.secret_access_key,
        params.session_token.clone(),
        None,
        "plano",
    );

    let settings = SigningSettings::default();
    let identity: Identity = credentials.into();

    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(&params.region)
        .name(&params.service)
        .time(SystemTime::now())
        .settings(settings)
        .build()
        .map_err(|e| AwsError::SigningError(format!("Failed to build signing params: {}", e)))?;

    let host = params.headers.get("host").cloned().unwrap_or_default();
    let url = if params.query_string.is_empty() {
        format!("https://{}{}", host, params.uri)
    } else {
        format!("https://{}{}?{}", host, params.uri, params.query_string)
    };

    let header_pairs: Vec<(String, String)> = params
        .headers
        .iter()
        .filter(|(k, _)| {
            let k = k.as_str();
            k != "host" && k != "x-amz-date" && k != "x-amz-security-token"
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let signable_body = SignableBody::Bytes(&params.payload);

    let signable_request = SignableRequest::new(
        &params.method,
        &url,
        header_pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        signable_body,
    )
    .map_err(|e| AwsError::SigningError(format!("Failed to create signable request: {}", e)))?;

    let signing_output = sign(signable_request, &signing_params.into())
        .map_err(|e| AwsError::SigningError(format!("Failed to sign request: {}", e)))?;

    let mut authorization = String::new();
    let mut amz_date = String::new();
    let (instructions, _) = signing_output.into_parts();
    for (name, value) in instructions.headers() {
        match name {
            "authorization" => authorization = value.to_string(),
            "x-amz-date" => amz_date = value.to_string(),
            _ => {}
        }
    }

    if authorization.is_empty() {
        return Err(AwsError::SigningError(
            "Authorization header not produced by signing".to_string(),
        ));
    }

    Ok((authorization, amz_date))
}

#[cfg(not(feature = "aws-sigv4"))]
pub fn sign_request(_params: SigV4Params) -> Result<(String, String), AwsError> {
    Err(AwsError::SigningError(
        "aws-signing feature not enabled".to_string(),
    ))
}

#[cfg(all(test, feature = "aws-sigv4"))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_sign_request_produces_authorization() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "host".to_string(),
            "bedrock-runtime.us-east-1.amazonaws.com".to_string(),
        );
        headers.insert("content-type".to_string(), "application/json".to_string());

        let params = SigV4Params {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: Some("test-session-token".to_string()),
            region: "us-east-1".to_string(),
            service: "bedrock-runtime".to_string(),
            method: "POST".to_string(),
            uri: "/model/test-model/converse".to_string(),
            query_string: String::new(),
            headers,
            payload: b"{}".to_vec(),
        };

        let result = sign_request(params);
        assert!(result.is_ok(), "sign_request should succeed");

        let (authorization, amz_date) = result.unwrap();
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256"),
            "Should use AWS4-HMAC-SHA256 algorithm"
        );
        assert!(!amz_date.is_empty(), "amz_date should not be empty");
    }
}
