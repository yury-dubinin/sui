// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
};
use sui_open_rpc::MethodRouting;

#[derive(Debug, Clone)]
pub struct RpcRouter {
    routes: HashMap<String, MethodRouting>,
    route_to_methods: HashSet<String>,
    disable_routing: bool,
}

impl RpcRouter {
    pub fn new(routes: HashMap<String, MethodRouting>, disable_routing: bool) -> Self {
        let route_to_methods = routes.values().map(|v| v.route_to.clone()).collect();

        Self {
            routes,
            route_to_methods,
            disable_routing,
        }
    }

    pub fn route<'c, 'a: 'c, 'b: 'c>(
        &'a self,
        method: &'b str,
        version: Option<&str>,
        client_addr: SocketAddr,
    ) -> &'c str {
        // Reject direct access to the old methods
        if self.route_to_methods.contains(method) {
            return "INVALID_ROUTING";
        }
        match method {
            // unconditional re-routing
            "execute_transaction_block" => "monitored_execute_transaction_block",
            // conditional re-routing
            _ => {
                if self.disable_routing {
                    method
                } else {
                    // Modify the method name if routing is enabled
                    match (version, route) {
                        (Some(v), Some(route)) if route.matches(v) => route.route_to.as_str(),
                        _ => method,
                    }
                }
            }
        }
    }
}
