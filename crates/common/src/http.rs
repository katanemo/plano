use crate::{
    errors::ClientError,
    stats::{Gauge, IncrementingMetric},
};
use derivative::Derivative;
use log::debug;
use proxy_wasm::traits::Context;
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap, fmt::Debug, time::Duration};

#[derive(Derivative, Serialize)]
#[derivative(Debug)]
pub struct CallArgs<'a> {
    upstream: &'a str,
    path: &'a str,
    headers: Vec<(&'a str, &'a str)>,
    #[derivative(Debug = "ignore")]
    body: Option<&'a [u8]>,
    trailers: Vec<(&'a str, &'a str)>,
    timeout: Duration,
}

impl<'a> CallArgs<'a> {
    pub fn new(
        upstream: &'a str,
        path: &'a str,
        headers: Vec<(&'a str, &'a str)>,
        body: Option<&'a [u8]>,
        trailers: Vec<(&'a str, &'a str)>,
        timeout: Duration,
    ) -> Self {
        CallArgs {
            upstream,
            path,
            headers,
            body,
            trailers,
            timeout,
        }
    }
}

pub trait Client: Context {
    type CallContext: Debug;

    fn http_call(
        &self,
        call_args: CallArgs,
        call_context: Self::CallContext,
    ) -> Result<u32, ClientError> {
        debug!(
            "dispatching http call with args={:?} context={:?}",
            call_args, call_context
        );

        match self.dispatch_http_call(
            call_args.upstream,
            call_args.headers,
            call_args.body,
            call_args.trailers,
            call_args.timeout,
        ) {
            Ok(id) => {
                self.add_call_context(id, call_context);
                Ok(id)
            }
            Err(status) => Err(ClientError::DispatchError {
                upstream_name: String::from(call_args.upstream),
                path: String::from(call_args.path),
                internal_status: status,
            }),
        }
    }

    fn add_call_context(&self, id: u32, call_context: Self::CallContext) {
        let callouts = self.callouts();
        if callouts.borrow_mut().insert(id, call_context).is_some() {
            log::warn!(
                "Duplicate http call with id={}, previous context overwritten",
                id
            );
        }
        self.active_http_calls().increment(1);
    }

    fn callouts(&self) -> &RefCell<HashMap<u32, Self::CallContext>>;

    fn active_http_calls(&self) -> &Gauge;
}
