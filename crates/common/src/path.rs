use std::collections::{HashMap, HashSet};
use url::Url;
use urlencoding;

use crate::configuration::Parameter;

pub fn replace_params_in_path(
    path: &str,
    tool_params: &HashMap<String, String>,
    prompt_target_params: &[Parameter],
) -> Result<(String, String, HashMap<String, String>), String> {
    let mut query_string_replaced = String::new();
    let mut current_param = String::new();
    let mut vars_replaced = HashSet::new();
    let mut params: HashMap<String, String> = HashMap::new();

    let mut in_param = false;
    for c in path.chars() {
        if c == '{' {
            in_param = true;
        } else if c == '}' {
            in_param = false;
            if let Some(value) = tool_params.get(&current_param) {
                let value = urlencoding::encode(value);
                query_string_replaced.push_str(value.into_owned().as_str());
                vars_replaced.insert(current_param.clone());
            } else {
                return Err(format!("Missing value for parameter `{}`", current_param));
            }
            current_param.clear();
        } else if in_param {
            current_param.push(c);
        } else {
            query_string_replaced.push(c);
        }
    }

    // add the remaining params in path
    for (param_name, value) in tool_params.iter() {
        let value = urlencoding::encode(value).into_owned();
        if !vars_replaced.contains(param_name) {
            vars_replaced.insert(param_name.clone());
            if query_string_replaced.contains("?") {
                query_string_replaced.push_str(&format!("&{}={}", param_name, value));
            } else {
                query_string_replaced.push_str(&format!("?{}={}", param_name, value));
            }
            params.insert(param_name.clone(), value);
        }
    }

    // add default values
    for param in prompt_target_params.iter() {
        if !vars_replaced.contains(&param.name) {
            if let Some(default_val) = &param.default {
                params.insert(param.name.clone(), default_val.clone());
                if query_string_replaced.contains("?") {
                    query_string_replaced.push_str(&format!("&{}={}", param.name, default_val));
                } else {
                    query_string_replaced.push_str(&format!("?{}={}", param.name, default_val));
                }
            }
        }
    }

    let parsed_uri = Url::parse("http://dummy.com").unwrap();
    let parsed_uri = parsed_uri
        .join(&query_string_replaced)
        .map_err(|e| e.to_string())?;
    let query_string = parsed_uri.query().unwrap_or("");
    let path_uri = parsed_uri.path();

    Ok((path_uri.to_string(), query_string.to_string(), params))
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use crate::configuration::Parameter;

    #[test]
    fn test_replace_path() {
        let path = "/cluster.open-cluster-management.io/v1/managedclusters/{cluster_name}";
        let params = vec![
            ("cluster_name".to_string(), "test1".to_string()),
            ("hello".to_string(), "hello world".to_string()),
        ]
        .into_iter()
        .collect();
        let prompt_target_params = vec![Parameter {
            name: "country".to_string(),
            parameter_type: None,
            description: "test target".to_string(),
            required: None,
            enum_values: None,
            default: Some("US".to_string()),
            in_path: None,
            format: None,
        }];

        let out_params: HashMap<String, String> = vec![
            ("country".to_string(), "US".to_string()),
            ("hello".to_string(), "hello%20world".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok((
                "/cluster.open-cluster-management.io/v1/managedclusters/test1".to_string(),
                "hello=hello%20world&country=US".to_string(),
                out_params
            ))
        );

        let out_params = HashMap::new();
        let prompt_target_params = vec![];
        let path = "/cluster.open-cluster-management.io/v1/managedclusters";
        let params = vec![].into_iter().collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok((
                "/cluster.open-cluster-management.io/v1/managedclusters".to_string(),
                "".to_string(),
                out_params
            ))
        );

        let path = "/foo/{bar}/baz";
        let params = vec![("bar".to_string(), "qux".to_string())]
            .into_iter()
            .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok(("/foo/qux/baz".to_string(), "".to_string(), HashMap::new()))
        );

        let path = "/foo/{bar}/baz/{qux}";
        let params = vec![
            ("bar".to_string(), "qux".to_string()),
            ("qux".to_string(), "quux".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok((
                "/foo/qux/baz/quux".to_string(),
                "".to_string(),
                HashMap::new()
            ))
        );

        let path = "/foo/{bar}/baz/{qux}?hello=world";
        let params = vec![
            ("bar".to_string(), "qux".to_string()),
            ("qux".to_string(), "quux".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok((
                "/foo/qux/baz/quux".to_string(),
                "hello=world".to_string(),
                HashMap::new()
            ))
        );

        let path = "/foo/{bar}/baz/{qux}?hello={hello}";
        let params = vec![
            ("bar".to_string(), "qux".to_string()),
            ("qux".to_string(), "quux".to_string()),
            ("hello".to_string(), "hello world".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Ok((
                "/foo/qux/baz/quux".to_string(),
                "hello=hello%20world".to_string(),
                HashMap::new()
            ))
        );

        let path = "/foo/{bar}/baz/{qux}";
        let params = vec![("bar".to_string(), "qux".to_string())]
            .into_iter()
            .collect();
        assert_eq!(
            super::replace_params_in_path(path, &params, &prompt_target_params),
            Err("Missing value for parameter `qux`".to_string())
        );
    }
}
