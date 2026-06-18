// Copyright (c) Zing CDN
// SPDX-License-Identifier: Apache-2.0

/// Safe u64 multiplication and division with u128 intermediate precision.
module zing_cdn::utils;

/// Computes (a * b) / c using u128 intermediate values to avoid overflow.
/// Aborts on division by zero or result > u64::max.
public fun mul_div(a: u64, b: u64, c: u64): u64 {
    let r = u128_mul_div((a as u128), (b as u128), (c as u128));
    assert!(r <= (18446744073709551615u64 as u128), 101);
    (r as u64)
}

/// Computes (a * b) / c with u128. Preserves maximum precision by doing
/// division before multiplication on the larger operand.
public fun u128_mul_div(a: u128, b: u128, c: u128): u128 {
    let (mut big, mut small) = if (a >= b) { (a, b) } else { (b, a) };
    assert!(c > 0, 101);
    big / c * small + big % c * small / c
}
