// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;
use sui_json_rpc::name_service;

#[test]
fn test_name_service_outputs() {
    let domain = name_service::Domain::from_str("@test").unwrap().to_string();
    assert_eq!(domain == "test.sui", true);

    let domain = name_service::Domain::from_str("test.sui").unwrap().to_string();
    assert_eq!(domain == "test.sui", true);

    let domain = name_service::Domain::from_str("@test@sui").unwrap().to_string();
    assert_eq!(domain == "test.sui", true);

    let domain = name_service::Domain::from_str("test@test@sui").unwrap().to_string();
    assert_eq!(domain == "test.test.sui", true);

    let domain = name_service::Domain::from_str("test.test").unwrap().to_string();
    assert_eq!(domain == "test.test.sui", true);

    let domain = name_service::Domain::from_str("test@test").unwrap().to_string();
    assert_eq!(domain == "test.test.sui", true);

    let domain = name_service::Domain::from_str("test@sui").unwrap().to_string();
    assert_eq!(domain == "test.sui", true);

    let domain = name_service::Domain::from_str("test").unwrap().to_string();
    assert_eq!(domain == "test.sui", true);
}

#[test]
fn test_tld_formatted_sld_outputs(){
    let domain = name_service::Domain::from_str("@sui").unwrap().to_string();
    assert_eq!(domain == "sui.sui", true);

    let domain = name_service::Domain::from_str("@sui@sui").unwrap().to_string();
    assert_eq!(domain == "sui.sui", true);
}
