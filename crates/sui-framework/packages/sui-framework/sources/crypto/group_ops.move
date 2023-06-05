// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Generic Move and native functions for group operations.
module sui::group_ops {

    use std::vector;

    friend sui::bls12381;
    friend sui::ristretto255;

    // TODO: remove before merging
    use std::debug;

    const EInvalidInput: u64 = 0;
    const EInvalidBufferLength: u64 = 1;

    /////////////////////////////////////////////////////
    ////// Generic functions for group operations. //////

    // The caller provides a type identifer that should match the types of enum [Groups] in group_ops.rs.

    // General wrapper for all group elements.
    struct Element<phantom T> has store, copy, drop {
        bytes: vector<u8>,
    }

    public fun bytes<G>(e: &Element<G>): &vector<u8> {
        &e.bytes
    }
    
    public fun equal<G>(e1: &Element<G>, e2: &Element<G>): bool {
        // TODO: Remove before merging
        debug::print(&e1.bytes);
        debug::print(&e2.bytes);
        e1.bytes == e2.bytes
    }

    // Fails if the bytes are not a valid group element and 'is_trusted' is false.
    public(friend) fun from_bytes<G>(type: u8, bytes: &vector<u8>, is_trusted: bool): Element<G> {
        assert!(is_trusted || internal_validate(type, bytes), EInvalidInput);
        Element<G> { bytes: *bytes }
    }

    public(friend) fun add<G>(type: u8, e1: &Element<G>, e2: &Element<G>): Element<G> {
        Element<G> { bytes: internal_add(type, &e1.bytes, &e2.bytes) }
    }

    public(friend) fun sub<G>(type: u8, e1: &Element<G>, e2: &Element<G>): Element<G> {
        Element<G> { bytes: internal_sub(type, &e1.bytes, &e2.bytes) }
    }

    public(friend) fun mul<S, G>(type: u8, scalar: &Element<S>, e: &Element<G>): Element<G> {
        Element<G> { bytes: internal_mul(type, &scalar.bytes, &e.bytes) }
    }

    // Fails if scalar = 0. Else returns 1/scalar * e.
    public(friend) fun div<S, G>(type: u8, scalar: &Element<S>, e: &Element<G>): Element<G> {
        Element<G> { bytes: internal_div(type, &scalar.bytes, &e.bytes) }
    }

    public(friend) fun hash_to<G>(type: u8, m: &vector<u8>): Element<G> {
        Element<G> { bytes: internal_hash_to(type, m) }
    }

    public(friend) fun multi_scalar_multiplication<S, G>(type: u8, scalars: &vector<Element<S>>, elements: &vector<Element<G>>): Element<G> {
        assert!(vector::length(scalars) == vector::length(elements), EInvalidInput);
        assert!(vector::length(scalars) > 0, EInvalidInput);
        assert!(vector::length(elements) <= 32, EInvalidInput); // TODO: other limit?

        let scalars_bytes = vector::empty<u8>();
        let elements_bytes = vector::empty<u8>();
        let i = 0;
        while (i < vector::length(scalars)) {
            let scalar_vec = *vector::borrow(scalars, i);
            vector::append(&mut scalars_bytes, scalar_vec.bytes);
            let element_vec = *vector::borrow(elements, i);
            vector::append(&mut elements_bytes, element_vec.bytes);
            i = i + 1;
        };
        Element<G> { bytes: internal_multi_scalar_mul(type, &scalars_bytes, &elements_bytes) }
    }

    public(friend) fun pairing<G1, G2, G3>(type: u8, e1: &Element<G1>, e2: &Element<G2>): Element<G3> {
        Element<G3> { bytes: internal_pairing(type, &e1.bytes, &e2.bytes) }
    }

    //////////////////////////////
    ////// Native functions //////

    // The following functions do *not* check whether the right types are used (e.g., Risretto255's scalar is used with
    // Ristrertto255's G). The caller to the above functions is responsible for that.

    // 'type' specifies the type of all elements.
    native fun internal_validate(type: u8, bytes: &vector<u8>): bool;
    native fun internal_add(type: u8, e1: &vector<u8>, e2: &vector<u8>): vector<u8>;
    native fun internal_sub(type: u8, e1: &vector<u8>, e2: &vector<u8>): vector<u8>;

    // 'type' represents the type of e2, and the type of e1 is determined automatically from e2. e1 is a scalar
    // and e2 is a group/scalar element.
    native fun internal_mul(type: u8, e1: &vector<u8>, e2: &vector<u8>): vector<u8>;
    native fun internal_div(type: u8, e1: &vector<u8>, e2: &vector<u8>): vector<u8>;

    // TODO: do we want to support any DST for BLS12-381?
    native fun internal_hash_to(type: u8, m: &vector<u8>): vector<u8>;
    native fun internal_multi_scalar_mul(type: u8, scalars: &vector<u8>, elements: &vector<u8>): vector<u8>;

    // 'type' represents the type of e1, and the rest are determined automatically from e1.
    native fun internal_pairing(type:u8, e1: &vector<u8>, e2: &vector<u8>): vector<u8>;
    // TODO: multi_pairing like msm? or vectorized pairing/add/mul/etc?

    // Helper function for encoding a given u64 number as bytes in a given buffer.
    public(friend) fun set_as_prefix(x: u64, big_endian: bool, buffer: &mut vector<u8>) {
        let buffer_len = vector::length(buffer);
        assert!(buffer_len > 7, EInvalidBufferLength);
        let i = 0;
        while (i < 8) {
            let curr_byte = x % 0x100;
            let position = if (big_endian) { buffer_len - i - 1 } else { i };
            let curr_element = vector::borrow_mut(buffer, position);
            *curr_element = (curr_byte as u8);
            x = x >> 8;
            i = i + 1;
        };
    }
}