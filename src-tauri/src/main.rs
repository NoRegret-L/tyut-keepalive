// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    tyut_keepalive_lib::run()
}
