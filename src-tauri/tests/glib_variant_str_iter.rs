// SPDX-License-Identifier: MPL-2.0

//! Regression for RUSTSEC-2024-0429, including optimized Linux builds.
#![cfg(target_os = "linux")]
use glib::{prelude::*, Variant};

#[test]
fn string_iterator_matches_forward_reverse_and_skip_contracts() {
    let variant = Variant::array_from_iter::<String>(
        ["first", "", "校园", "last"].map(|s| s.to_variant()),
    );
    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some("first"));
    assert_eq!(iter.next_back(), Some("last"));
    assert_eq!(iter.nth(1), Some("校园"));
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
    assert_eq!(variant.array_iter_str().unwrap().nth_back(1), Some("校园"));
    assert_eq!(variant.array_iter_str().unwrap().last(), Some("last"));
    assert_eq!(variant.array_iter_str().unwrap().collect::<Vec<_>>(),
               vec!["first", "", "校园", "last"]);
    assert_eq!(variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(),
               vec!["last", "校园", "", "first"]);
}
