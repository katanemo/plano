use crate::metrics::Metrics;
use crate::stream_context::StreamContext;
use common::configuration::Configuration;
use common::configuration::Overrides;
use common::dlp::DlpScanner;
use common::http::Client;
use common::llm_providers::LlmProviders;
use common::ratelimit;
use common::stats::Gauge;
use log::{debug, trace};
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

/// Identifies what a pending callout was for
#[derive(Debug)]
pub enum CallContext {
    /// Polling for blocked projects
    BudgetBlocked,
}

pub struct FilterContext {
    metrics: Rc<Metrics>,
    // callouts stores token_id to request mapping that we use during #on_http_call_response to match the response to the request.
    callouts: RefCell<HashMap<u32, CallContext>>,
    llm_providers: Option<Rc<LlmProviders>>,
    overrides: Rc<Option<Overrides>>,
    dlp_scanner: Option<Rc<DlpScanner>>,
    /// Set of project IDs that have exceeded their spending limits.
    /// Populated by periodic polling of GET /budget/blocked.
    blocked_projects: Rc<RefCell<HashSet<String>>>,
}

impl FilterContext {
    pub fn new() -> FilterContext {
        FilterContext {
            callouts: RefCell::new(HashMap::new()),
            metrics: Rc::new(Metrics::new()),
            llm_providers: None,
            overrides: Rc::new(None),
            dlp_scanner: None,
            blocked_projects: Rc::new(RefCell::new(HashSet::new())),
        }
    }

    /// Poll brightstaff for the list of budget-blocked projects
    fn poll_budget_blocked(&self) {
        let headers = vec![
            (":method", "GET"),
            (":path", "/budget/blocked"),
            (":authority", "bright_staff"),
        ];

        match self.dispatch_http_call(
            "bright_staff",
            headers,
            None,
            vec![],
            Duration::from_secs(2),
        ) {
            Ok(token_id) => {
                self.callouts
                    .borrow_mut()
                    .insert(token_id, CallContext::BudgetBlocked);
                trace!("budget blocked poll dispatched, token_id={}", token_id);
            }
            Err(e) => {
                debug!("failed to dispatch budget blocked poll: {:?}", e);
            }
        }
    }
}

impl Client for FilterContext {
    type CallContext = CallContext;

    fn callouts(&self) -> &RefCell<HashMap<u32, Self::CallContext>> {
        &self.callouts
    }

    fn active_http_calls(&self) -> &Gauge {
        &self.metrics.active_http_calls
    }
}

// RootContext allows the Rust code to reach into the Envoy Config
impl RootContext for FilterContext {
    fn on_configure(&mut self, _: usize) -> bool {
        let config_bytes = self
            .get_plugin_configuration()
            .expect("Arch config cannot be empty");

        let config: Configuration = match serde_yaml::from_slice(&config_bytes) {
            Ok(config) => config,
            Err(err) => panic!("Invalid arch config \"{:?}\"", err),
        };

        ratelimit::ratelimits(Some(config.ratelimits.unwrap_or_default()));
        self.overrides = Rc::new(config.overrides);

        // Initialize DLP scanner if configured
        if let Some(ref dlp_config) = config.dlp {
            if dlp_config.enabled {
                self.dlp_scanner = Some(Rc::new(DlpScanner::new(dlp_config)));
                log::info!(
                    "DLP scanner initialized with {} patterns",
                    dlp_config.patterns.len()
                );
            }
        }

        match config.model_providers.try_into() {
            Ok(llm_providers) => self.llm_providers = Some(Rc::new(llm_providers)),
            Err(err) => panic!("{err}"),
        }

        true
    }

    fn create_http_context(&self, context_id: u32) -> Option<Box<dyn HttpContext>> {
        trace!(
            "create_http_context called with context_id: {:?}",
            context_id
        );

        Some(Box::new(StreamContext::new(
            Rc::clone(&self.metrics),
            Rc::clone(
                self.llm_providers
                    .as_ref()
                    .expect("LLM Providers must exist when Streams are being created"),
            ),
            Rc::clone(&self.overrides),
            self.dlp_scanner.clone(),
            Rc::clone(&self.blocked_projects),
        )))
    }

    fn get_type(&self) -> Option<ContextType> {
        Some(ContextType::HttpContext)
    }

    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        self.set_tick_period(Duration::from_secs(1));
        true
    }

    fn on_tick(&mut self) {
        // Poll for budget-blocked projects every tick (1s)
        self.poll_budget_blocked();
    }
}

impl Context for FilterContext {
    fn on_http_call_response(
        &mut self,
        token_id: u32,
        _num_headers: usize,
        body_size: usize,
        _num_trailers: usize,
    ) {
        trace!("on_http_call_response called with token_id: {:?}", token_id);

        let callout_data = self.callouts.borrow_mut().remove(&token_id);

        match callout_data {
            Some(CallContext::BudgetBlocked) => {
                // Parse the blocked projects response
                if let Some(body) = self.get_http_call_response_body(0, body_size) {
                    if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(blocked) = parsed.get("blocked").and_then(|b| b.as_array()) {
                            let mut new_set = HashSet::new();
                            for id in blocked {
                                if let Some(s) = id.as_str() {
                                    new_set.insert(s.to_string());
                                }
                            }
                            let count = new_set.len();
                            *self.blocked_projects.borrow_mut() = new_set;
                            if count > 0 {
                                debug!("updated blocked projects: {} blocked", count);
                            }
                        }
                    }
                }
            }
            None => {
                if let Some(status) = self.get_http_call_response_header(":status") {
                    trace!("response status: {:?}", status);
                };
            }
        }
    }
}
