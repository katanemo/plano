use crate::metrics::Metrics;
use crate::stream_context::StreamContext;
use common::configuration::Configuration;
use common::configuration::Overrides;
use common::http::Client;
use common::llm_providers::LlmProviders;
use common::ratelimit;
use common::stats::Gauge;
use log::trace;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

#[derive(Debug)]
pub struct CallContext {}

#[derive(Debug)]
pub struct FilterContext {
    metrics: Rc<Metrics>,
    // callouts stores token_id to request mapping that we use during #on_http_call_response to match the response to the request.
    callouts: RefCell<HashMap<u32, CallContext>>,
    llm_providers: Option<Rc<LlmProviders>>,
    overrides: Rc<Option<Overrides>>,
}

impl FilterContext {
    pub fn new() -> FilterContext {
        FilterContext {
            callouts: RefCell::new(HashMap::new()),
            metrics: Rc::new(Metrics::new()),
            llm_providers: None,
            overrides: Rc::new(None),
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
        )))
    }

    fn get_type(&self) -> Option<ContextType> {
        Some(ContextType::HttpContext)
    }

    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        self.set_tick_period(Duration::from_secs(1));
        true
    }
}

impl Context for FilterContext {
    fn on_http_call_response(
        &mut self,
        token_id: u32,
        _num_headers: usize,
        _body_size: usize,
        _num_trailers: usize,
    ) {
        trace!("on_http_call_response called with token_id: {:?}", token_id);

        let _callout_data = self
            .callouts
            .borrow_mut()
            .remove(&token_id)
            .expect("invalid token_id");

        if let Some(status) = self.get_http_call_response_header(":status") {
            trace!("response status: {:?}", status);
        };
    }
}
