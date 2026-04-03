use crate::configuration;
use configuration::{Limit, Ratelimit, TimeUnit};
use governor::{DefaultKeyedRateLimiter, InsufficientCapacity, Quota};
use log::debug;
use std::fmt::Display;
use std::num::{NonZero, NonZeroU32};
use std::sync::RwLock;
use std::{collections::HashMap, sync::OnceLock};

pub type RatelimitData = RwLock<RatelimitMap>;

pub fn ratelimits(ratelimits_config: Option<Vec<Ratelimit>>) -> &'static RatelimitData {
    static RATELIMIT_DATA: OnceLock<RatelimitData> = OnceLock::new();
    RATELIMIT_DATA.get_or_init(|| {
        RwLock::new(RatelimitMap::new(
            ratelimits_config.expect("The initialization call has to have passed a config"),
        ))
    })
}

// The Data Structure is laid out in the following way:
// Provider -> Hash { Header -> Limit }.
// If the Header used to configure the given Limit:
//   a) Has None value, then there will be N Limit keyed by the Header value.
//   b) Has Some() value, then there will be 1 Limit keyed by the empty string.
// It would have been nicer to use a non-keyed limit for b). However, the type system made that option a nightmare.
pub struct RatelimitMap {
    datastore: HashMap<String, HashMap<configuration::Header, DefaultKeyedRateLimiter<String>>>,
}

// This version of Header demands that the user passes a header value to match on.
#[derive(Debug, Clone)]
pub struct Header {
    pub key: String,
    pub value: String,
}

impl Display for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<Header> for configuration::Header {
    fn from(header: Header) -> Self {
        Self {
            key: header.key,
            value: Some(header.value),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("exceeded limit provider={provider}, selector={selector}, tokens_used={tokens_used}")]
    ExceededLimit {
        provider: String,
        selector: Header,
        tokens_used: NonZeroU32,
    },
}

impl RatelimitMap {
    // n.b new is private so that the only access to the Ratelimits can be done via the static
    // reference inside a RwLock via ratelimit::ratelimits().
    fn new(ratelimits_config: Vec<Ratelimit>) -> Self {
        let mut new_ratelimit_map = RatelimitMap {
            datastore: HashMap::new(),
        };
        for ratelimit_config in ratelimits_config {
            let limit = DefaultKeyedRateLimiter::keyed(get_quota(ratelimit_config.limit));

            match new_ratelimit_map.datastore.get_mut(&ratelimit_config.model) {
                Some(limits) => match limits.get_mut(&ratelimit_config.selector) {
                    Some(_) => {
                        log::error!(
                            "repeated selector for model '{}'. Selectors per provider must be unique, skipping duplicate",
                            ratelimit_config.model
                        );
                    }
                    None => {
                        limits.insert(ratelimit_config.selector, limit);
                    }
                },
                None => {
                    // The provider has not been seen before.
                    // Insert the provider and a new HashMap with the specified limit
                    let new_hash_map = HashMap::from([(ratelimit_config.selector, limit)]);
                    new_ratelimit_map
                        .datastore
                        .insert(ratelimit_config.model, new_hash_map);
                }
            }
        }
        new_ratelimit_map
    }

    #[allow(unused)]
    pub fn check_limit(
        &self,
        provider: String,
        selector: Header,
        tokens_used: NonZeroU32,
    ) -> Result<(), Error> {
        debug!(
            "Checking limit for provider={}, with selector={:?}, consuming tokens={:?}",
            provider, selector, tokens_used
        );

        let provider_limits = match self.datastore.get(&provider) {
            None => {
                // No limit configured for this provider, hence ok.
                return Ok(());
            }
            Some(limit) => limit,
        };

        let mut config_selector = configuration::Header::from(selector.clone());

        let (limit, limit_key) = match provider_limits.get(&config_selector) {
            // This is a specific limit, i.e one that was configured with both key, and value.
            // Therefore, the key for the internal limit does not matter, and hence the empty string is always returned.
            Some(limit) => (limit, String::from("")),
            None => {
                // Unwrap is ok here because we _know_ the value exists.
                let header_key = config_selector.value.take().unwrap();
                // Search for less specific limit, i.e, one that was configured without a value, therefore every Header
                // value has its own key in the internal limit.
                match provider_limits.get(&config_selector) {
                    Some(limit) => (limit, header_key),
                    // No limit for that header key, value pair exists within that provider limits.
                    None => {
                        return Ok(());
                    }
                }
            }
        };

        match limit.check_key_n(&limit_key, tokens_used) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(InsufficientCapacity(_)) => Err(Error::ExceededLimit {
                provider,
                selector,
                tokens_used,
            }),
        }
    }
}

fn get_quota(limit: Limit) -> Quota {
    let tokens = NonZero::new(limit.tokens).expect("Limit's tokens must be positive");
    match limit.unit {
        TimeUnit::Second => Quota::per_second(tokens),
        TimeUnit::Minute => Quota::per_minute(tokens),
        TimeUnit::Hour => Quota::per_hour(tokens),
        TimeUnit::Day => {
            let per_hour = limit.tokens.saturating_div(24).max(1);
            Quota::per_hour(NonZero::new(per_hour).expect("per_hour must be positive"))
        }
    }
}

// The following tests are inside the ratelimit module in order to access RatelimitMap::new() in order to provide
// different configuration values per test.
#[test]
fn non_existent_provider_is_ok() {
    let ratelimits_config = vec![Ratelimit {
        model: String::from("provider"),
        selector: configuration::Header {
            key: String::from("only-key"),
            value: None,
        },
        limit: Limit {
            tokens: 100,
            unit: TimeUnit::Minute,
        },
    }];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    assert!(ratelimits
        .check_limit(
            String::from("non-existent-provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(5000).unwrap(),
        )
        .is_ok())
}

#[test]
fn non_existent_key_is_ok() {
    let ratelimits_config = vec![Ratelimit {
        model: String::from("provider"),
        selector: configuration::Header {
            key: String::from("only-key"),
            value: None,
        },
        limit: Limit {
            tokens: 100,
            unit: TimeUnit::Minute,
        },
    }];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(5000).unwrap(),
        )
        .is_ok())
}

#[test]
fn specific_limit_does_not_catch_non_specific_value() {
    let ratelimits_config = vec![Ratelimit {
        model: String::from("provider"),
        selector: configuration::Header {
            key: String::from("key"),
            value: Some(String::from("value")),
        },
        limit: Limit {
            tokens: 200,
            unit: TimeUnit::Second,
        },
    }];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("key"),
                value: String::from("not-the-correct-value"),
            },
            NonZero::new(5000).unwrap(),
        )
        .is_ok())
}

#[test]
fn specific_limit_is_hit() {
    let ratelimits_config = vec![Ratelimit {
        model: String::from("provider"),
        selector: configuration::Header {
            key: String::from("key"),
            value: Some(String::from("value")),
        },
        limit: Limit {
            tokens: 200,
            unit: TimeUnit::Hour,
        },
    }];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(5000).unwrap(),
        )
        .is_err())
}

#[test]
fn non_specific_key_has_different_limits_for_different_values() {
    let ratelimits_config = vec![Ratelimit {
        model: String::from("provider"),
        selector: configuration::Header {
            key: String::from("only-key"),
            value: None,
        },
        limit: Limit {
            tokens: 100,
            unit: TimeUnit::Hour,
        },
    }];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    // Value1 takes 50.
    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("only-key"),
                value: String::from("value1"),
            },
            NonZero::new(50).unwrap(),
        )
        .is_ok());

    // value2 takes 60 because it has its own 100 limit
    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("only-key"),
                value: String::from("value2"),
            },
            NonZero::new(60).unwrap(),
        )
        .is_ok());

    // However value1 cannot take more than 100 per hour which 50+70 = 120
    assert!(ratelimits
        .check_limit(
            String::from("provider"),
            Header {
                key: String::from("only-key"),
                value: String::from("value1"),
            },
            NonZero::new(70).unwrap(),
        )
        .is_err())
}

#[test]
fn different_provider_can_have_different_limits_with_the_same_keys() {
    let ratelimits_config = vec![
        Ratelimit {
            model: String::from("first_provider"),
            selector: configuration::Header {
                key: String::from("key"),
                value: Some(String::from("value")),
            },
            limit: Limit {
                tokens: 100,
                unit: TimeUnit::Hour,
            },
        },
        Ratelimit {
            model: String::from("second_provider"),
            selector: configuration::Header {
                key: String::from("key"),
                value: Some(String::from("value")),
            },
            limit: Limit {
                tokens: 200,
                unit: TimeUnit::Hour,
            },
        },
    ];

    let ratelimits = RatelimitMap::new(ratelimits_config);

    assert!(ratelimits
        .check_limit(
            String::from("first_provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(100).unwrap(),
        )
        .is_ok());

    assert!(ratelimits
        .check_limit(
            String::from("second_provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(200).unwrap(),
        )
        .is_ok());

    assert!(ratelimits
        .check_limit(
            String::from("first_provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(1).unwrap(),
        )
        .is_err());

    assert!(ratelimits
        .check_limit(
            String::from("second_provider"),
            Header {
                key: String::from("key"),
                value: String::from("value"),
            },
            NonZero::new(1).unwrap(),
        )
        .is_err());
}

// These tests use the publicly exposed static singleton, thus the same configuration is used in every test.
// If more tests are written here, move the initial call out of the test.
#[cfg(test)]
mod test {
    use crate::configuration;

    use super::ratelimits;
    use configuration::{Limit, Ratelimit, TimeUnit};
    use std::num::NonZero;
    use std::thread;

    #[test]
    fn make_ratelimits_optional() {
        let ratelimits_config = Vec::new();

        // Initialize in the main thread.
        ratelimits(Some(ratelimits_config));
    }

    #[test]
    fn different_threads_have_same_ratelimit_data_structure() {
        let ratelimits_config = Some(vec![Ratelimit {
            model: String::from("provider"),
            selector: configuration::Header {
                key: String::from("key"),
                value: Some(String::from("value")),
            },
            limit: Limit {
                tokens: 200,
                unit: TimeUnit::Hour,
            },
        }]);

        // Initialize in the main thread.
        ratelimits(ratelimits_config);

        // Use the singleton in a different thread.
        thread::spawn(|| {
            let ratelimits = ratelimits(None);

            assert!(ratelimits
                .read()
                .unwrap()
                .check_limit(
                    String::from("provider"),
                    super::Header {
                        key: String::from("key"),
                        value: String::from("value"),
                    },
                    NonZero::new(5000).unwrap(),
                )
                .is_err())
        });
    }
}
