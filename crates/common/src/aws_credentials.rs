use crate::configuration::AwsCredentialsConfig;
use crate::errors::AwsError;

pub fn get_credentials_from_config(
    config: &AwsCredentialsConfig,
) -> Result<(String, String, Option<String>), AwsError> {
    let access_key_id = config
        .access_key_id
        .as_ref()
        .ok_or_else(|| AwsError::CredentialError("AWS_ACCESS_KEY_ID not found".to_string()))?
        .clone();

    let secret_access_key = config
        .secret_access_key
        .as_ref()
        .ok_or_else(|| AwsError::CredentialError("AWS_SECRET_ACCESS_KEY not found".to_string()))?
        .clone();

    Ok((
        access_key_id,
        secret_access_key,
        config.session_token.clone(),
    ))
}
