// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0
use crate::object_runtime::ObjectRuntime;
use fastcrypto::error::{FastCryptoError, FastCryptoResult};
use fastcrypto::groups::{
    bls12381 as bls, GroupElement, HashToGroupElement, MultiScalarMul, Pairing,
};
use fastcrypto::serde_helpers::ToFromByteArray;
use move_binary_format::errors::PartialVMResult;
use move_vm_runtime::native_functions::NativeContext;
use move_vm_types::{
    loaded_data::runtime_types::Type,
    natives::function::NativeResult,
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;
use std::collections::VecDeque;

pub const INVALID_INPUT_ERROR: u64 = 0;
pub const NOT_SUPPORTED_ERROR: u64 = 1;

fn is_supported(context: &NativeContext) -> bool {
    context
        .extensions()
        .get::<ObjectRuntime>()
        .protocol_config
        .enable_group_ops_native_functions()
}

// Next should be aligned with the relevant Move modules.
#[repr(u8)]
enum Groups {
    BLS12381Scalar = 0,
    BLS12381G1 = 1,
    BLS12381G2 = 2,
    BLS12381GT = 3,
}

impl Groups {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Groups::BLS12381Scalar),
            1 => Some(Groups::BLS12381G1),
            2 => Some(Groups::BLS12381G2),
            3 => Some(Groups::BLS12381GT),
            _ => None,
        }
    }
}

fn parse<G: ToFromByteArray<S>, const S: usize>(e: &[u8]) -> FastCryptoResult<G> {
    G::from_byte_array(e.try_into().map_err(|_| FastCryptoError::InvalidInput)?)
}

// Binary operations with 2 different types.
fn binary_op_diff<
    G1: ToFromByteArray<S1>,
    G2: ToFromByteArray<S2>,
    const S1: usize,
    const S2: usize,
>(
    op: impl Fn(G1, G2) -> FastCryptoResult<G2>,
    a1: &[u8],
    a2: &[u8],
) -> FastCryptoResult<Vec<u8>> {
    let e1 = parse::<G1, S1>(a1)?;
    let e2 = parse::<G2, S2>(a2)?;
    let result = op(e1, e2)?;
    Ok(result.to_byte_array().to_vec())
}

// Binary operations with the same type.
fn binary_op<G: ToFromByteArray<S>, const S: usize>(
    op: impl Fn(G, G) -> FastCryptoResult<G>,
    a1: &[u8],
    a2: &[u8],
) -> FastCryptoResult<Vec<u8>> {
    binary_op_diff::<G, G, S, S>(op, a1, a2)
}

// TODO: Since in many cases more than one group operation will be performed in a single
// transaction, it might be worth caching the affine representation of the group elements and use
// them to save conversions.

pub fn internal_validate(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 2);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let bytes_ref = pop_arg!(args, VectorRef);
    let bytes = bytes_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381Scalar) => {
            parse::<bls::Scalar, { bls::Scalar::BYTE_LENGTH }>(&bytes).is_ok()
        }
        Some(Groups::BLS12381G1) => {
            parse::<bls::G1Element, { bls::G1Element::BYTE_LENGTH }>(&bytes).is_ok()
        }
        Some(Groups::BLS12381G2) => {
            parse::<bls::G2Element, { bls::G2Element::BYTE_LENGTH }>(&bytes).is_ok()
        }
        Some(Groups::BLS12381GT) => {
            parse::<bls::GTElement, { bls::GTElement::BYTE_LENGTH }>(&bytes).is_ok()
        }
        _ => false,
    };

    Ok(NativeResult::ok(cost, smallvec![Value::bool(result)]))
}

pub fn internal_add(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let e2_ref = pop_arg!(args, VectorRef);
    let e2 = e2_ref.as_bytes_ref();
    let e1_ref = pop_arg!(args, VectorRef);
    let e1 = e1_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381Scalar) => {
            binary_op::<bls::Scalar, { bls::Scalar::BYTE_LENGTH }>(|a, b| Ok(a + b), &e1, &e2)
        }
        Some(Groups::BLS12381G1) => {
            binary_op::<bls::G1Element, { bls::G1Element::BYTE_LENGTH }>(|a, b| Ok(a + b), &e1, &e2)
        }
        Some(Groups::BLS12381G2) => {
            binary_op::<bls::G2Element, { bls::G2Element::BYTE_LENGTH }>(|a, b| Ok(a + b), &e1, &e2)
        }
        Some(Groups::BLS12381GT) => {
            binary_op::<bls::GTElement, { bls::GTElement::BYTE_LENGTH }>(|a, b| Ok(a + b), &e1, &e2)
        }
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong or inputs are invalid.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

pub fn internal_sub(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let e2_ref = pop_arg!(args, VectorRef);
    let e2 = e2_ref.as_bytes_ref();
    let e1_ref = pop_arg!(args, VectorRef);
    let e1 = e1_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381Scalar) => {
            binary_op::<bls::Scalar, { bls::Scalar::BYTE_LENGTH }>(|a, b| Ok(a - b), &e1, &e2)
        }
        Some(Groups::BLS12381G1) => {
            binary_op::<bls::G1Element, { bls::G1Element::BYTE_LENGTH }>(|a, b| Ok(a - b), &e1, &e2)
        }
        Some(Groups::BLS12381G2) => {
            binary_op::<bls::G2Element, { bls::G2Element::BYTE_LENGTH }>(|a, b| Ok(a - b), &e1, &e2)
        }
        Some(Groups::BLS12381GT) => {
            binary_op::<bls::GTElement, { bls::GTElement::BYTE_LENGTH }>(|a, b| Ok(a - b), &e1, &e2)
        }
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong or inputs are invalid.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

pub fn internal_mul(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let e2_ref = pop_arg!(args, VectorRef);
    let e2 = e2_ref.as_bytes_ref();
    let e1_ref = pop_arg!(args, VectorRef);
    let e1 = e1_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381Scalar) => {
            binary_op::<bls::Scalar, { bls::Scalar::BYTE_LENGTH }>(|a, b| Ok(b * a), &e1, &e2)
        }
        Some(Groups::BLS12381G1) => binary_op_diff::<
            bls::Scalar,
            bls::G1Element,
            { bls::Scalar::BYTE_LENGTH },
            { bls::G1Element::BYTE_LENGTH },
        >(|a, b| Ok(b * a), &e1, &e2),
        Some(Groups::BLS12381G2) => binary_op_diff::<
            bls::Scalar,
            bls::G2Element,
            { bls::Scalar::BYTE_LENGTH },
            { bls::G2Element::BYTE_LENGTH },
        >(|a, b| Ok(b * a), &e1, &e2),
        Some(Groups::BLS12381GT) => binary_op_diff::<
            bls::Scalar,
            bls::GTElement,
            { bls::Scalar::BYTE_LENGTH },
            { bls::GTElement::BYTE_LENGTH },
        >(|a, b| Ok(b * a), &e1, &e2),
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong or inputs are invalid.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

pub fn internal_div(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let e2_ref = pop_arg!(args, VectorRef);
    let e2 = e2_ref.as_bytes_ref();
    let e1_ref = pop_arg!(args, VectorRef);
    let e1 = e1_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381Scalar) => {
            binary_op::<bls::Scalar, { bls::Scalar::BYTE_LENGTH }>(|a, b| b / a, &e1, &e2)
        }
        Some(Groups::BLS12381G1) => binary_op_diff::<
            bls::Scalar,
            bls::G1Element,
            { bls::Scalar::BYTE_LENGTH },
            { bls::G1Element::BYTE_LENGTH },
        >(|a, b| b / a, &e1, &e2),
        Some(Groups::BLS12381G2) => binary_op_diff::<
            bls::Scalar,
            bls::G2Element,
            { bls::Scalar::BYTE_LENGTH },
            { bls::G2Element::BYTE_LENGTH },
        >(|a, b| b / a, &e1, &e2),
        Some(Groups::BLS12381GT) => binary_op_diff::<
            bls::Scalar,
            bls::GTElement,
            { bls::Scalar::BYTE_LENGTH },
            { bls::GTElement::BYTE_LENGTH },
        >(|a, b| b / a, &e1, &e2),
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong, inputs are invalid, or a=0.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

pub fn internal_hash_to(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 2);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let m_ref = pop_arg!(args, VectorRef);
    let m = m_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381G1) => Ok(bls::G1Element::hash_to_group_element(&m)
            .to_byte_array()
            .to_vec()),
        Some(Groups::BLS12381G2) => Ok(bls::G2Element::hash_to_group_element(&m)
            .to_byte_array()
            .to_vec()),
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong or inputs are invalid.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

fn multi_scalar_mul<G, const S1: usize, const S2: usize>(
    scalars: &Vec<u8>,
    points: &Vec<u8>,
) -> FastCryptoResult<Vec<u8>>
where
    G: GroupElement + ToFromByteArray<S1> + MultiScalarMul,
    G::ScalarType: ToFromByteArray<S2>,
{
    if points.len() % S1 != 0 || scalars.len() % S2 != 0 || points.len() / S1 != scalars.len() / S2
    {
        return Err(FastCryptoError::InvalidInput);
    }
    let points = points
        .chunks(S1)
        .map(parse::<G, { S1 }>)
        .collect::<Result<Vec<_>, _>>();
    let scalars = scalars
        .chunks(S2)
        .map(parse::<G::ScalarType, { S2 }>)
        .collect::<Result<Vec<_>, _>>();

    if let (Ok(scalars), Ok(points)) = (scalars, points) {
        let r = G::multi_scalar_mul(&scalars, &points)
            .expect("Already checked the lengths of the vectors");
        Ok(r.to_byte_array().to_vec())
    } else {
        Err(FastCryptoError::InvalidInput)
    }
}

pub fn internal_multi_scalar_mul(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let elements_ref = pop_arg!(args, VectorRef);
    let elements = elements_ref.as_bytes_ref();
    let scalars_ref = pop_arg!(args, VectorRef);
    let scalars = scalars_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    // TODO: can potentially improve performance when some of the points are the generator.
    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381G1) => multi_scalar_mul::<
            bls::G1Element,
            { bls::G1Element::BYTE_LENGTH },
            { bls::Scalar::BYTE_LENGTH },
        >(scalars.as_ref(), elements.as_ref()),
        Some(Groups::BLS12381G2) => multi_scalar_mul::<
            bls::G2Element,
            { bls::G2Element::BYTE_LENGTH },
            { bls::Scalar::BYTE_LENGTH },
        >(scalars.as_ref(), elements.as_ref()),
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong or inputs are invalid.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}

pub fn internal_pairing(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    debug_assert!(ty_args.is_empty());
    debug_assert!(args.len() == 3);

    // TODO: charge fees
    let cost = context.gas_used();
    if !is_supported(context) {
        return Ok(NativeResult::err(cost, NOT_SUPPORTED_ERROR));
    }

    let e2_ref = pop_arg!(args, VectorRef);
    let e2 = e2_ref.as_bytes_ref();
    let e1_ref = pop_arg!(args, VectorRef);
    let e1 = e1_ref.as_bytes_ref();
    let group_type = pop_arg!(args, u8);

    let result = match Groups::from_u8(group_type) {
        Some(Groups::BLS12381G1) => parse::<bls::G1Element, { bls::G1Element::BYTE_LENGTH }>(&e1)
            .and_then(|e1| {
                parse::<bls::G2Element, { bls::G2Element::BYTE_LENGTH }>(&e2).map(|e2| {
                    let e3 = e1.pairing(&e2);
                    e3.to_byte_array().to_vec()
                })
            }),
        _ => Err(FastCryptoError::InvalidInput),
    };

    match result {
        Ok(bytes) => Ok(NativeResult::ok(cost, smallvec![Value::vector_u8(bytes)])),
        // Since all Element<G> are validated on construction, this error should never happen unless the requested type is wrong.
        Err(_) => Ok(NativeResult::err(cost, INVALID_INPUT_ERROR)),
    }
}
